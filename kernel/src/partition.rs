//! 通用分区块设备（GPT / MBR / raw ext4 整盘）。
//!
//! 竞赛评测镜像可能是整盘 raw ext4、带 GPT 分区表或普通 MBR。本模块提供：
//!
//! - 边界受限的 [`PartitionBlockDevice`] 视图（只读属性继承父设备）；
//! - 从父设备自动探测分区布局（raw ext4 / GPT / MBR）；
//! - Linux 风格分区命名（`vda1` / `mmcblk1p1` / `sda1`）。
//!
//! 不支持扩展分区链（CodePlan C2）。探测使用 `MemoryBlockDevice` 构造的
//! fixture，不需要真实 `.img`。

use alloc::{string::String, sync::Arc, vec, vec::Vec};

#[cfg(debug_assertions)]
use crate::block::MemoryBlockDevice;

use crate::{
    block::{self, BlockDevice, BlockError},
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
};

const PARTITION_LOCK: LockClass = LockClass::new("partition.device", LockRank::Vfs, 12);

const LBA_SIZE: usize = 512;

const EXT4_SUPER_MAGIC: u16 = 0xef53;
const EXT4_SUPER_OFFSET: u64 = 1024;
const EXT4_SUPER_MAGIC_OFFSET: u64 = 56;

const MBR_SIGNATURE: u16 = 0xaa55;
const MBR_PARTITION_OFFSET: usize = 446;
const MBR_PARTITION_ENTRY_SIZE: usize = 16;
const MBR_PARTITION_COUNT: usize = 4;
/// GPT 保护分区类型（不解析为真实分区）。
const MBR_GPT_PROTECTIVE: u8 = 0xee;

const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
const GPT_HEADER_SIZE: usize = 92;
const GPT_ENTRY_SIZE: usize = 128;
const MAX_GPT_ENTRIES: usize = 128;
const MAX_GPT_ENTRIES_BYTES: usize = MAX_GPT_ENTRIES * GPT_ENTRY_SIZE;
/// GPT 分区属性 bit 60 = read-only。
const GPT_READONLY_ATTR: u64 = 1 << 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartitionError {
    AddressOverflow,
    BadBlockSize,
    BadPartitionTable,
    Block(BlockError),
    OutOfMemory,
    Unsupported,
}

impl From<BlockError> for PartitionError {
    fn from(error: BlockError) -> Self {
        Self::Block(error)
    }
}

/// 探测出的分区描述（尚无最终注册名）。
///
/// `number` 是分区表里的真实序号（MBR 槽位 / GPT 项号，均 1 起始）。
/// 扫描器跳过空项后仍需保留真实序号，注册时不能按返回列表的下标重排
/// （否则稀疏分区表会把第 3 分区压缩命名为 p1/p2）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartitionSpec {
    pub number: u32,
    pub first_lba: u64,
    pub block_count: u64,
    pub read_only: bool,
}

/// 父设备上一段有界的分区视图。
pub struct PartitionBlockDevice {
    parent: Arc<dyn BlockDevice>,
    first_lba: u64,
    block_count: u64,
    read_only: bool,
}

impl PartitionBlockDevice {
    /// 构造有界分区视图。`first_lba + block_count` 必须不越过父设备且
    /// 不发生 LBA 溢出；只读性继承父设备（或由调用方按分区表属性置位）。
    pub fn new(
        parent: Arc<dyn BlockDevice>,
        first_lba: u64,
        block_count: u64,
        read_only: bool,
    ) -> Result<Self, PartitionError> {
        let parent_blocks = parent.block_count();
        let end = first_lba
            .checked_add(block_count)
            .ok_or(PartitionError::AddressOverflow)?;
        if block_count == 0 {
            return Err(PartitionError::BadPartitionTable);
        }
        if end > parent_blocks {
            return Err(PartitionError::AddressOverflow);
        }
        let read_only = read_only || parent.is_read_only();
        Ok(Self {
            parent,
            first_lba,
            block_count,
            read_only,
        })
    }
}

impl BlockDevice for PartitionBlockDevice {
    fn block_size(&self) -> usize {
        self.parent.block_size()
    }

    fn block_count(&self) -> u64 {
        self.block_count
    }

    fn read_block(&self, block: u64, output: &mut [u8]) -> Result<(), BlockError> {
        if output.len() != self.block_size() {
            return Err(BlockError::BufferTooSmall);
        }
        if block >= self.block_count {
            return Err(BlockError::OutOfRange);
        }
        let lba = self
            .first_lba
            .checked_add(block)
            .ok_or(BlockError::AddressOverflow)?;
        self.parent.read_block(lba, output)
    }

    fn write_block(&self, block: u64, input: &[u8]) -> Result<(), BlockError> {
        if self.read_only {
            return Err(BlockError::DeviceReadOnly);
        }
        if input.len() != self.block_size() {
            return Err(BlockError::BufferTooSmall);
        }
        if block >= self.block_count {
            return Err(BlockError::OutOfRange);
        }
        let lba = self
            .first_lba
            .checked_add(block)
            .ok_or(BlockError::AddressOverflow)?;
        self.parent.write_block(lba, input)
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn is_partition(&self) -> bool {
        true
    }
}

/// 扫描父设备的分区布局。
///
/// 识别顺序：raw ext4 整盘 → GPT → MBR。raw ext4 整盘返回一个覆盖全盘的
/// 分区；GPT/MBR 返回各自的主分区。无分区表时返回空列表。
pub fn scan_partitions(
    parent: &Arc<dyn BlockDevice>,
) -> Result<Vec<PartitionSpec>, PartitionError> {
    let block_size = parent.block_size();
    if block_size < LBA_SIZE {
        return Err(PartitionError::BadBlockSize);
    }
    let parent_blocks = parent.block_count();
    if parent_blocks == 0 {
        return Err(PartitionError::BadPartitionTable);
    }

    // raw ext4 整盘：超级块魔数直接出现在设备头部。
    if device_is_ext4(parent)? {
        return Ok(vec![PartitionSpec {
            number: 1,
            first_lba: 0,
            block_count: parent_blocks,
            read_only: parent.is_read_only(),
        }]);
    }

    if let Some(specs) = scan_gpt(parent, parent_blocks)? {
        return Ok(specs);
    }

    let mut lba0 = [0_u8; LBA_SIZE];
    block::read_at(parent, 0, &mut lba0)?;
    if has_mbr_signature(&lba0) {
        return Ok(scan_mbr(&lba0, parent, parent_blocks));
    }

    Ok(Vec::new())
}

/// 把 `base_name` 磁盘上探测出的每个分区注册为独立块设备。
///
/// 返回成功注册的设备名。重名（如已注册）静默跳过。
pub fn register_partitions(
    base_name: &str,
    device: &Arc<dyn BlockDevice>,
) -> Result<Vec<String>, PartitionError> {
    let specs = scan_partitions(device)?;
    let mut registered = Vec::new();
    for spec in specs.iter() {
        let name = partition_name(base_name, spec.number);
        let partition = Arc::new(PartitionBlockDevice::new(
            Arc::clone(device),
            spec.first_lba,
            spec.block_count,
            spec.read_only,
        )?);
        match block::register_device(&name, partition as Arc<dyn BlockDevice>) {
            Ok(()) => registered.push(name),
            Err(BlockError::InvalidArgument) => {
                // 同名设备已存在（例如整盘 raw ext4 场景下 vda 与 vda1 并存）。
            }
            Err(error) => return Err(PartitionError::Block(error)),
        }
    }
    Ok(registered)
}

/// 已注册分区名索引：基础磁盘名 → 该盘的分区设备名列表。
static PARTITIONS_BY_DISK: IrqSpinLock<Vec<(String, Vec<String>)>> =
    IrqSpinLock::new_with_class(Vec::new(), PARTITION_LOCK);

/// 扫描注册表中所有基础磁盘（非分区设备，如 `vda`/`ram0`/`mmcblk1`）并注册
/// 各自的分区。必须在 `fs::initialize()` 之前调用，使分区设备在 `/dev` 树
/// 可见；同时记录 `基础盘 → 分区名` 索引供存储选择降级用。
///
/// 单个磁盘扫描失败（无分区布局是常态）静默跳过；返回成功注册的分区设备数。
pub fn register_all_partitions() -> usize {
    let base = match block::registered_devices() {
        Ok(snapshot) => snapshot,
        Err(_) => return 0,
    };
    let mut by_disk: Vec<(String, Vec<String>)> = Vec::new();
    let mut total = 0;
    for entry in base {
        let device = entry.device();
        if device.is_partition() {
            continue;
        }
        match register_partitions(entry.name(), &device) {
            Ok(names) if !names.is_empty() => {
                total += names.len();
                by_disk.push((String::from(entry.name()), names));
            }
            Ok(_) | Err(_) => {}
        }
    }
    *PARTITIONS_BY_DISK.lock() = by_disk;
    total
}

/// 返回某基础磁盘已注册的分区设备名列表（如 `["mmcblk1p1", "mmcblk1p2"]`）。
/// 基础盘本身是 raw ext4 时也会有一条 `{base}1`（与整盘并存，选择逻辑优先
/// 整盘）。无分区或未注册时返回空列表。
pub fn partitions_of(base_name: &str) -> Vec<String> {
    PARTITIONS_BY_DISK
        .lock()
        .iter()
        .find(|(name, _)| name == base_name)
        .map(|(_, names)| names.clone())
        .unwrap_or_default()
}

/// Linux 风格的分区命名：`mmcblk*` 使用 `pN` 后缀，其余直接拼接数字。
pub fn partition_name(disk: &str, partition: u32) -> String {
    if disk.starts_with("mmcblk") {
        alloc::format!("{}p{}", disk, partition)
    } else {
        alloc::format!("{}{}", disk, partition)
    }
}

fn device_is_ext4(parent: &Arc<dyn BlockDevice>) -> Result<bool, PartitionError> {
    let mut magic = [0_u8; 2];
    let read = block::read_at(
        parent,
        EXT4_SUPER_OFFSET + EXT4_SUPER_MAGIC_OFFSET,
        &mut magic,
    )?;
    if read != magic.len() {
        return Ok(false);
    }
    Ok(u16::from_le_bytes(magic) == EXT4_SUPER_MAGIC)
}

fn has_mbr_signature(lba0: &[u8; LBA_SIZE]) -> bool {
    u16::from_le_bytes([lba0[510], lba0[511]]) == MBR_SIGNATURE
}

/// 解析 GPT 分区表。返回 `Ok(Some(specs))` 表示这是 GPT 且解析成功；
/// `Ok(None)` 表示头部没有 GPT 签名（不是 GPT）；CRC 或越界错误返回
/// `Err`（表已损坏，不静默忽略）。
fn scan_gpt(
    parent: &Arc<dyn BlockDevice>,
    parent_blocks: u64,
) -> Result<Option<Vec<PartitionSpec>>, PartitionError> {
    if parent_blocks < 3 {
        return Ok(None);
    }
    let mut header = [0_u8; LBA_SIZE];
    block::read_at(parent, LBA_SIZE as u64, &mut header)?;
    if &header[..8] != GPT_SIGNATURE {
        return Ok(None);
    }

    let header_size = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
    if header_size < GPT_HEADER_SIZE || header_size > LBA_SIZE {
        return Err(PartitionError::BadPartitionTable);
    }
    let stored_crc = u32::from_le_bytes(header[16..20].try_into().unwrap());
    // UEFI 规范：header CRC32 覆盖整个头部（全部 header_size 字节，含 size
    // 字段），计算时 CRC 字段清零。header_size 允许大于最小 92 字节
    // （保留字段/未来扩展），因此缓冲区按 LBA_SIZE 分配，避免越界 panic。
    let mut crc_buf = [0_u8; LBA_SIZE];
    crc_buf[..header_size].copy_from_slice(&header[..header_size]);
    crc_buf[16..20].copy_from_slice(&0_u32.to_le_bytes());
    if crc32(&crc_buf[..header_size]) != stored_crc {
        return Err(PartitionError::BadPartitionTable);
    }

    let entries_lba = u64::from_le_bytes(header[72..80].try_into().unwrap());
    let num_entries = u32::from_le_bytes(header[80..84].try_into().unwrap()) as usize;
    let entry_size = u32::from_le_bytes(header[84..88].try_into().unwrap()) as usize;
    if entry_size < GPT_ENTRY_SIZE || entry_size % 8 != 0 {
        return Err(PartitionError::BadPartitionTable);
    }
    if num_entries > MAX_GPT_ENTRIES {
        return Err(PartitionError::Unsupported);
    }
    let entries_bytes = num_entries
        .checked_mul(entry_size)
        .ok_or(PartitionError::AddressOverflow)?;
    if entries_bytes > MAX_GPT_ENTRIES_BYTES {
        return Err(PartitionError::Unsupported);
    }
    let entries_offset = entries_lba
        .checked_mul(LBA_SIZE as u64)
        .ok_or(PartitionError::AddressOverflow)?;

    let stored_entries_crc = u32::from_le_bytes(header[88..92].try_into().unwrap());
    let mut entries = Vec::new();
    entries
        .try_reserve(entries_bytes)
        .map_err(|_| PartitionError::OutOfMemory)?;
    entries.resize(entries_bytes, 0);
    block::read_at(parent, entries_offset, &mut entries)?;
    if crc32(&entries[..entries_bytes]) != stored_entries_crc {
        return Err(PartitionError::BadPartitionTable);
    }

    let mut specs = Vec::new();
    for index in 0..num_entries {
        let entry = &entries[index * entry_size..index * entry_size + GPT_ENTRY_SIZE];
        if entry[..16].iter().all(|byte| *byte == 0) {
            continue;
        }
        let first_lba = u64::from_le_bytes(entry[32..40].try_into().unwrap());
        let last_lba = u64::from_le_bytes(entry[40..48].try_into().unwrap());
        if last_lba < first_lba {
            return Err(PartitionError::BadPartitionTable);
        }
        let block_count = last_lba
            .checked_sub(first_lba)
            .and_then(|delta| delta.checked_add(1))
            .ok_or(PartitionError::AddressOverflow)?;
        let end = first_lba
            .checked_add(block_count)
            .ok_or(PartitionError::AddressOverflow)?;
        if end > parent_blocks {
            return Err(PartitionError::BadPartitionTable);
        }
        let attributes = u64::from_le_bytes(entry[48..56].try_into().unwrap());
        specs.push(PartitionSpec {
            number: (index + 1) as u32,
            first_lba,
            block_count,
            read_only: attributes & GPT_READONLY_ATTR != 0,
        });
    }
    Ok(Some(specs))
}

/// 解析普通 MBR 主分区（跳过空项与 GPT 保护项；越界项静默跳过）。
fn scan_mbr(
    lba0: &[u8; LBA_SIZE],
    parent: &Arc<dyn BlockDevice>,
    parent_blocks: u64,
) -> Vec<PartitionSpec> {
    let mut specs = Vec::new();
    for index in 0..MBR_PARTITION_COUNT {
        let offset = MBR_PARTITION_OFFSET + index * MBR_PARTITION_ENTRY_SIZE;
        let partition_type = lba0[offset + 4];
        if partition_type == 0 || partition_type == MBR_GPT_PROTECTIVE {
            continue;
        }
        let first_lba =
            u32::from_le_bytes(lba0[offset + 8..offset + 12].try_into().unwrap()) as u64;
        let sector_count =
            u32::from_le_bytes(lba0[offset + 12..offset + 16].try_into().unwrap()) as u64;
        if sector_count == 0 {
            continue;
        }
        let end = match first_lba.checked_add(sector_count) {
            Some(end) => end,
            None => continue,
        };
        if end > parent_blocks {
            continue;
        }
        specs.push(PartitionSpec {
            number: (index + 1) as u32,
            first_lba,
            block_count: sector_count,
            read_only: parent.is_read_only(),
        });
    }
    specs
}

/// IEEE 802.3 CRC32（GPT 表校验用）。
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320_u32 & mask);
        }
    }
    !crc
}

#[cfg(debug_assertions)]
pub fn verify() {
    use crate::block::MemoryBlockDevice;

    // 1) 命名规则。
    assert_eq!(partition_name("vda", 1), "vda1");
    assert_eq!(partition_name("mmcblk1", 1), "mmcblk1p1");
    assert_eq!(partition_name("sda", 2), "sda2");

    // 2) raw ext4 整盘：超级块魔数 → 一个覆盖全盘的分区。
    let raw = make_disk(64);
    let mut super_sector = [0_u8; 512];
    super_sector[56..58].copy_from_slice(&0xef53_u16.to_le_bytes());
    raw.write_block(2, &super_sector).expect("write ext4 magic");
    let raw_specs = scan_partitions(&Arc::clone(&raw)).expect("raw ext4 scan");
    assert_eq!(raw_specs, vec![PartitionSpec {
        number: 1,
        first_lba: 0,
        block_count: 64,
        read_only: false,
    }],);

    // 3) 正常 GPT：两个分区。
    let gpt = make_disk(64);
    install_gpt(&gpt, &[(34, 40, false), (50, 60, true)], GPT_HEADER_SIZE);
    let gpt_specs = scan_partitions(&Arc::clone(&gpt)).expect("gpt scan");
    assert_eq!(
        gpt_specs,
        vec![
            PartitionSpec {
                number: 1,
                first_lba: 34,
                block_count: 7,
                read_only: false,
            },
            PartitionSpec {
                number: 2,
                first_lba: 50,
                block_count: 11,
                read_only: true,
            },
        ],
        "GPT partitions must carry their read-only attribute",
    );

    // 4) GPT CRC 损坏 → BadPartitionTable。
    let corrupt = make_disk(64);
    install_gpt(&corrupt, &[(34, 40, false)], GPT_HEADER_SIZE);
    let mut bad_header = [0_u8; 512];
    corrupt
        .read_block(1, &mut bad_header)
        .expect("read gpt header");
    bad_header[16] ^= 0xff;
    corrupt
        .write_block(1, &bad_header)
        .expect("corrupt gpt header crc");
    assert_eq!(
        scan_partitions(&Arc::clone(&corrupt)),
        Err(PartitionError::BadPartitionTable),
        "corrupted GPT header CRC must be surfaced",
    );

    // 5) 分区越界：GPT 项超出父设备 → BadPartitionTable。
    let oob = make_disk(64);
    install_gpt(&oob, &[(50, 100, false)], GPT_HEADER_SIZE);
    assert_eq!(
        scan_partitions(&Arc::clone(&oob)),
        Err(PartitionError::BadPartitionTable),
        "GPT partition extending past the parent must be rejected",
    );

    // 6) 空分区表：无分区项的 GPT → 空结果。
    let empty = make_disk(64);
    install_gpt(&empty, &[], GPT_HEADER_SIZE);
    assert!(
        scan_partitions(&Arc::clone(&empty))
            .expect("empty gpt scan")
            .is_empty()
    );

    // 7) 普通 MBR：两个主分区（0xEE 保护项被跳过）。
    let mbr = make_disk(64);
    install_mbr(&mbr, &[(1, 20, 0x0c), (30, 15, 0x83)]);
    let mbr_specs = scan_partitions(&Arc::clone(&mbr)).expect("mbr scan");
    assert_eq!(mbr_specs, vec![
        PartitionSpec {
            number: 1,
            first_lba: 1,
            block_count: 20,
            read_only: false,
        },
        PartitionSpec {
            number: 2,
            first_lba: 30,
            block_count: 15,
            read_only: false,
        },
    ],);

    // 7b) 稀疏 MBR：中间槽位为空 → 分区保留真实序号（1 与 3，不是 1、2），
    //     注册名也必须是 vda1/vda3 而非按列表下标压缩成 vda1/vda2。
    let sparse_mbr = make_disk(64);
    install_mbr(&sparse_mbr, &[(1, 20, 0x0c), (0, 0, 0), (30, 15, 0x83)]);
    let sparse_specs = scan_partitions(&Arc::clone(&sparse_mbr)).expect("sparse mbr scan");
    assert_eq!(
        sparse_specs,
        vec![
            PartitionSpec {
                number: 1,
                first_lba: 1,
                block_count: 20,
                read_only: false,
            },
            PartitionSpec {
                number: 3,
                first_lba: 30,
                block_count: 15,
                read_only: false,
            },
        ],
        "empty MBR slot must not renumber later partitions",
    );
    let sparse_names =
        register_partitions("vda", &Arc::clone(&sparse_mbr)).expect("register sparse mbr");
    assert_eq!(
        sparse_names,
        vec![String::from("vda1"), String::from("vda3")],
        "registered names must keep real MBR slot numbers",
    );
    crate::block::unregister_device("vda1").expect("unregister sparse p1");
    crate::block::unregister_device("vda3").expect("unregister sparse p3");

    // 8) 超大 LBA：分区构造时溢出 → AddressOverflow。
    assert_eq!(
        PartitionBlockDevice::new(Arc::clone(&raw), u64::MAX - 1, 3, false).err(),
        Some(PartitionError::AddressOverflow),
    );

    // 9) 父设备读错误传播。
    let failing: Arc<dyn BlockDevice> = Arc::new(FailingReadDevice);
    assert_eq!(
        scan_partitions(&failing),
        Err(PartitionError::Block(BlockError::AddressOverflow)),
    );

    // 10) 只读继承：父设备只读 → 分区只读且写被拒绝。
    let ro_parent: Arc<dyn BlockDevice> = Arc::new(ReadOnlyParent {
        inner: Arc::clone(&raw),
    });
    let part = PartitionBlockDevice::new(ro_parent, 0, 8, false)
        .expect("read-only partition construction");
    assert!(part.is_read_only());
    let mut block = [0_u8; 512];
    assert_eq!(part.write_block(0, &block), Err(BlockError::DeviceReadOnly),);

    // 11) 分区读写：读映射到父设备偏移；越界拒绝。
    let base = make_disk(64);
    let mut first = [0_u8; 512];
    first[0..8].copy_from_slice(b"partdata");
    base.write_block(2, &first).expect("write parent block 2");
    let part =
        PartitionBlockDevice::new(Arc::clone(&base), 2, 10, false).expect("partition construction");
    let mut readback = [0_u8; 512];
    part.read_block(0, &mut readback).expect("partition read");
    assert_eq!(&readback[0..8], b"partdata");
    assert_eq!(
        part.read_block(10, &mut readback),
        Err(BlockError::OutOfRange),
    );
    let mut write = [0_u8; 512];
    write[0..7].copy_from_slice(b"written");
    part.write_block(0, &write).expect("partition write");
    let mut check = [0_u8; 512];
    base.read_block(2, &mut check).expect("parent block read");
    assert_eq!(&check[0..7], b"written");

    // 12) register_partitions 按命名规则注册。
    let reg_disk = make_disk(64);
    install_gpt(&reg_disk, &[(34, 40, false)], GPT_HEADER_SIZE);
    let names = register_partitions("vda", &Arc::clone(&reg_disk)).expect("register partitions");
    assert_eq!(names, vec![String::from("vda1")]);
    assert!(crate::block::open_device("vda1").is_some());
    crate::block::unregister_device("vda1").expect("unregister partition");
    let mmc_names =
        register_partitions("mmcblk1", &Arc::clone(&reg_disk)).expect("register mmc partitions");
    assert_eq!(mmc_names, vec![String::from("mmcblk1p1")]);
    crate::block::unregister_device("mmcblk1p1").expect("unregister mmc partition");

    // 13) header_size > 92（扩展头部）：CRC 覆盖全部 header_size 字节，
    //     不得因固定 92 字节缓冲越界 panic。
    let wide = make_disk(64);
    install_gpt(&wide, &[(34, 40, false)], 128);
    let wide_specs = scan_partitions(&Arc::clone(&wide)).expect("wide-header gpt scan");
    assert_eq!(
        wide_specs,
        vec![PartitionSpec {
            number: 1,
            first_lba: 34,
            block_count: 7,
            read_only: false,
        }],
        "GPT with header_size=128 must parse without panic",
    );

    // 14) header_size > LBA_SIZE：扫描器必须在触碰 CRC 前按 size 拒绝，
    //     返回 BadPartitionTable 而不是越界。
    let oversized = make_disk(64);
    install_gpt(&oversized, &[(34, 40, false)], LBA_SIZE + 16);
    assert_eq!(
        scan_partitions(&Arc::clone(&oversized)),
        Err(PartitionError::BadPartitionTable),
        "header_size above LBA_SIZE must be rejected",
    );

    crate::println!("C2 partition gate:");
    crate::println!("  raw ext4 whole disk : verified");
    crate::println!("  GPT (valid + CRC)   : verified");
    crate::println!("  GPT CRC corrupt     : verified");
    crate::println!("  partition out-of-bound: verified");
    crate::println!("  empty partition     : verified");
    crate::println!("  MBR                 : verified");
    crate::println!("  oversized LBA       : verified");
    crate::println!("  parent error prop   : verified");
    crate::println!("  read-only inherit   : verified");
    crate::println!("  partition naming    : verified");
    crate::println!("  sparse MBR (real #) : verified");
    crate::println!("  GPT wide header     : verified");
    crate::println!("  GPT oversized header: verified");
}

#[cfg(debug_assertions)]
fn make_disk(blocks: u64) -> Arc<dyn BlockDevice> {
    Arc::new(MemoryBlockDevice::new(512, blocks).expect("fixture disk"))
}

/// 供其他模块测试复用：写入一张标准 92 字节头部的 GPT 表。
#[cfg(debug_assertions)]
pub(crate) fn install_gpt_fixture(device: &Arc<dyn BlockDevice>, partitions: &[(u64, u64, bool)]) {
    install_gpt(device, partitions, GPT_HEADER_SIZE);
}

/// 写入合法 GPT 表（保护 MBR + LBA1 头部 + LBA2 分区项）。
/// `header_size` 控制头部 size 字段（允许 > 92 以覆盖超大头部场景）。
#[cfg(debug_assertions)]
fn install_gpt(device: &Arc<dyn BlockDevice>, partitions: &[(u64, u64, bool)], header_size: usize) {
    assert!(
        header_size >= GPT_HEADER_SIZE,
        "fixture header_size must be at least GPT_HEADER_SIZE",
    );
    let blocks = device.block_count();

    let mut mbr = [0_u8; 512];
    mbr[446 + 4] = MBR_GPT_PROTECTIVE;
    mbr[446 + 8..446 + 12].copy_from_slice(&1_u32.to_le_bytes());
    mbr[446 + 12..446 + 16].copy_from_slice(&(blocks as u32).saturating_sub(1).to_le_bytes());
    mbr[510..512].copy_from_slice(&MBR_SIGNATURE.to_le_bytes());
    device.write_block(0, &mbr).expect("write protective mbr");

    let num_entries = partitions.len();
    let mut entries = alloc::vec![0_u8; MAX_GPT_ENTRIES_BYTES];
    for (index, &(first, last, read_only)) in partitions.iter().enumerate() {
        let entry = &mut entries[index * GPT_ENTRY_SIZE..index * GPT_ENTRY_SIZE + GPT_ENTRY_SIZE];
        entry[..16].copy_from_slice(&[0xaf; 16]);
        entry[16..32].copy_from_slice(&[0xbe; 16]);
        entry[32..40].copy_from_slice(&first.to_le_bytes());
        entry[40..48].copy_from_slice(&last.to_le_bytes());
        if read_only {
            entry[48..56].copy_from_slice(&GPT_READONLY_ATTR.to_le_bytes());
        }
    }
    let entries_bytes = num_entries * GPT_ENTRY_SIZE;
    let entries_crc = crc32(&entries[..entries_bytes]);
    for (offset, chunk) in entries[..entries_bytes].chunks(LBA_SIZE).enumerate() {
        let mut sector = [0_u8; LBA_SIZE];
        sector[..chunk.len()].copy_from_slice(chunk);
        device
            .write_block(2 + offset as u64, &sector)
            .expect("write gpt entries");
    }

    let mut header = [0_u8; LBA_SIZE];
    header[..8].copy_from_slice(GPT_SIGNATURE);
    header[8..12].copy_from_slice(&0x0001_0000_u32.to_le_bytes());
    header[12..16].copy_from_slice(&(header_size as u32).to_le_bytes());
    header[24..32].copy_from_slice(&1_u64.to_le_bytes());
    header[32..40].copy_from_slice(&(blocks - 1).to_le_bytes());
    header[40..48].copy_from_slice(&34_u64.to_le_bytes());
    header[48..56].copy_from_slice(&(blocks - 2).to_le_bytes());
    header[56..72].copy_from_slice(&[0x11; 16]);
    header[72..80].copy_from_slice(&2_u64.to_le_bytes());
    header[80..84].copy_from_slice(&(num_entries as u32).to_le_bytes());
    header[84..88].copy_from_slice(&(GPT_ENTRY_SIZE as u32).to_le_bytes());
    header[88..92].copy_from_slice(&entries_crc.to_le_bytes());
    // header_size > 92 时填充扩展区，保证 CRC 覆盖到真实数据（不是全零）。
    for byte in header[GPT_HEADER_SIZE..header_size.min(LBA_SIZE)].iter_mut() {
        *byte = 0x5a;
    }
    // CRC 覆盖 min(header_size, LBA_SIZE) 字节；超 LBA_SIZE 的场景由扫描器在
    // 检查 size 时提前拒绝，CRC 值无关紧要，这里避免在 fixture 内越界。
    let crc_len = header_size.min(LBA_SIZE);
    let header_crc = crc32(&header[..crc_len]);
    header[16..20].copy_from_slice(&header_crc.to_le_bytes());
    device.write_block(1, &header).expect("write gpt header");
}

/// 写入普通 MBR（四项之一项可选 GPT 保护项）。
#[cfg(debug_assertions)]
fn install_mbr(device: &Arc<dyn BlockDevice>, partitions: &[(u32, u32, u8)]) {
    let mut mbr = [0_u8; 512];
    for (index, &(first, sectors, partition_type)) in partitions.iter().enumerate() {
        let offset = MBR_PARTITION_OFFSET + index * MBR_PARTITION_ENTRY_SIZE;
        mbr[offset] = 0x00; // not bootable
        mbr[offset + 4] = partition_type;
        mbr[offset + 8..offset + 12].copy_from_slice(&first.to_le_bytes());
        mbr[offset + 12..offset + 16].copy_from_slice(&sectors.to_le_bytes());
    }
    mbr[510..512].copy_from_slice(&MBR_SIGNATURE.to_le_bytes());
    device.write_block(0, &mbr).expect("write mbr");
}

/// 读取总是失败的测试设备（验证父设备错误传播）。
#[cfg(debug_assertions)]
struct FailingReadDevice;

#[cfg(debug_assertions)]
impl BlockDevice for FailingReadDevice {
    fn block_size(&self) -> usize {
        512
    }

    fn block_count(&self) -> u64 {
        8
    }

    fn read_block(&self, _block: u64, _output: &mut [u8]) -> Result<(), BlockError> {
        Err(BlockError::AddressOverflow)
    }

    fn write_block(&self, _block: u64, _input: &[u8]) -> Result<(), BlockError> {
        Err(BlockError::DeviceReadOnly)
    }
}

/// 只读测试设备（验证分区只读属性继承）。
#[cfg(debug_assertions)]
struct ReadOnlyParent {
    inner: Arc<dyn BlockDevice>,
}

#[cfg(debug_assertions)]
impl BlockDevice for ReadOnlyParent {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn read_block(&self, block: u64, output: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, output)
    }

    fn write_block(&self, block: u64, input: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, input)
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
