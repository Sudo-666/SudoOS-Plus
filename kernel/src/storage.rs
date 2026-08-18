//! 公共竞赛存储层。
//!
//! 评测镜像所在的分区块设备由平台决定：
//!
//! - QEMU VirtIO  → `vda`
//! - VisionFive 2 TF 卡 → `mmcblk1`
//! - LS2K1000 U-Boot 内存盘 → `ram0`
//! - LS2K1000 原生 U 盘 → `sda`（后续）
//!
//! 本模块把“选哪块设备 + 是否 ext4 + 挂到 `/mnt/sdcard`”从具体设备名中
//! 解耦出来，统一由 `sudoos.contest.dev=<name>` 启动参数或自动扫描驱动
//! （CodePlan C1）。OSCOMP runner 不再依赖硬编码的 `/dev/vda`。

use alloc::{string::String, sync::Arc, vec, vec::Vec};

use crate::{
    block::{self, BlockDevice, BlockError},
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
};

const STATE_LOCK: LockClass = LockClass::new("storage.state", LockRank::Vfs, 3);

#[cfg(debug_assertions)]
use crate::block::MemoryBlockDevice;

/// ext4 超级块魔数位于文件系统内偏移 1024 + 56。
const EXT4_SUPER_MAGIC: u16 = 0xef53;
const EXT4_SUPER_OFFSET: u64 = 1024;
const EXT4_SUPER_MAGIC_OFFSET: u64 = 56;

/// 已选中的竞赛存储设备。
#[derive(Clone)]
pub struct SelectedStorage {
    name: String,
    device: Arc<dyn BlockDevice>,
}

impl SelectedStorage {
    /// 注册表设备名（如 `vda` / `mmcblk1` / `ram0`，不含 `/dev/` 前缀）。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 底层块设备（供后续安装逻辑复用同一 `Arc`）。
    pub fn device(&self) -> Arc<dyn BlockDevice> {
        Arc::clone(&self.device)
    }
}

/// 竞赛存储选择配置（来自 bootargs）。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContestStorageConfig {
    /// `sudoos.contest.dev=<name>` 指定的设备名。`None` 表示自动扫描。
    pub device_name: Option<String>,
    /// `sudoos.contest.required=1`：找不到竞赛存储必须明确失败，不静默降级
    /// preliminary（LS2K1000 USB 竞赛镜像路径用）。
    pub required: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageError {
    /// 设备上不存在 ext4 超级块魔数。
    NotExt4,
    /// 底层块设备 I/O 失败。
    Block(BlockError),
    /// 元数据内存分配失败。
    OutOfMemory,
}

impl From<BlockError> for StorageError {
    fn from(error: BlockError) -> Self {
        Self::Block(error)
    }
}

/// 当前已挂载的竞赛存储（未挂载时为 `None`）。
static SELECTED: IrqSpinLock<Option<SelectedStorage>> =
    IrqSpinLock::new_with_class(None, STATE_LOCK);

/// 从 bootargs 中解析 `sudoos.contest.dev=<name>` 与
/// `sudoos.contest.required=1`。两个键可同时出现；未知键忽略。
pub fn config_from_bootargs(args: Option<&str>) -> ContestStorageConfig {
    let Some(args) = args else {
        return ContestStorageConfig::default();
    };
    let mut device_name = None;
    let mut required = false;
    for word in args.split_whitespace() {
        if let Some(value) = word.strip_prefix("sudoos.contest.dev=") {
            if !value.is_empty() {
                device_name = Some(String::from(value));
            }
        } else if let Some(value) = word.strip_prefix("sudoos.contest.required=") {
            required = value == "1" || value.eq_ignore_ascii_case("true");
        }
    }
    ContestStorageConfig {
        device_name,
        required,
    }
}

/// 按 CodePlan C1 选择顺序解析设备名：
///
/// 1. `sudoos.contest.dev=<name>` 显式指定；
/// 2. 自动扫描注册表中第一个含 ext4 的设备；
/// 3. 兼容现有 `vda`（与自动扫描重叠，作为最终兜底）；
/// 4. 找不到时安全返回 `Ok(None)`，不 panic。
///
/// 打印验收日志 `CONTEST00` / `CONTEST01`。
pub fn select_device(
    config: &ContestStorageConfig,
) -> Result<Option<SelectedStorage>, StorageError> {
    match &config.device_name {
        Some(name) => {
            crate::println!("CONTEST00 requested-device={}", name);
            match block::open_device(name) {
                Some(device) => {
                    if device_is_ext4(&device)? {
                        crate::println!("CONTEST01 selected-device={}", name);
                        Ok(Some(SelectedStorage {
                            name: String::from(name),
                            device,
                        }))
                    } else if let Some(partition) = select_ext4_partition(name)? {
                        crate::println!("CONTEST01 selected-device={}", partition.name);
                        Ok(Some(partition))
                    } else {
                        Err(StorageError::NotExt4)
                    }
                }
                None => {
                    crate::println!("CONTEST01 selected-device=none");
                    Ok(None)
                }
            }
        }
        None => {
            crate::println!("CONTEST00 requested-device=auto");
            let snapshot = block::registered_devices()?;
            let mut candidates: Vec<(String, Arc<dyn BlockDevice>)> = Vec::new();
            candidates
                .try_reserve(snapshot.len())
                .map_err(|_| StorageError::OutOfMemory)?;
            for entry in snapshot {
                candidates.push((String::from(entry.name()), entry.device()));
            }
            let selected = select_from_candidates(config, &candidates)?;
            if let Some(storage) = selected {
                crate::println!("CONTEST01 selected-device={}", storage.name);
                return Ok(Some(storage));
            }
            // 兼容现有 vda：自动扫描理论上已覆盖，这里仅作最终兜底。
            if let Some(device) = block::open_device("vda") {
                if device_is_ext4(&device)? {
                    crate::println!("CONTEST01 selected-device=vda");
                    return Ok(Some(SelectedStorage {
                        name: String::from("vda"),
                        device,
                    }));
                }
            }
            crate::println!("CONTEST01 selected-device=none");
            Ok(None)
        }
    }
}

/// 纯选择算法（不依赖全局注册表，便于单元测试）。
///
/// `candidates` 为 `(设备名, 块设备)` 列表；按给定顺序扫描。
/// 显式指定且不存在时返回 `Ok(None)`；显式指定但非 ext4 时返回
/// `Err(NotExt4)`；自动扫描无匹配时返回 `Ok(None)`。
fn select_from_candidates(
    config: &ContestStorageConfig,
    candidates: &[(String, Arc<dyn BlockDevice>)],
) -> Result<Option<SelectedStorage>, StorageError> {
    match &config.device_name {
        Some(name) => {
            let Some((_, device)) = candidates.iter().find(|(candidate, _)| candidate == name)
            else {
                return Ok(None);
            };
            if !device_is_ext4(device)? {
                return Err(StorageError::NotExt4);
            }
            Ok(Some(SelectedStorage {
                name: String::from(name),
                device: Arc::clone(device),
            }))
        }
        None => {
            for (name, device) in candidates {
                if device_is_ext4(device)? {
                    return Ok(Some(SelectedStorage {
                        name: name.clone(),
                        device: Arc::clone(device),
                    }));
                }
            }
            Ok(None)
        }
    }
}

/// 显式指定设备整盘非 ext4 时，自动降级到它的第一个 ext4 分区
/// （如 `mmcblk1` → `mmcblk1p1`、`vda` → `vda1`）。分区设备由
/// `partition::register_all_partitions` 在启动期注册并索引。
fn select_ext4_partition(base_name: &str) -> Result<Option<SelectedStorage>, StorageError> {
    for partition_name in crate::partition::partitions_of(base_name) {
        let Some(device) = block::open_device(&partition_name) else {
            continue;
        };
        if device_is_ext4(&device)? {
            return Ok(Some(SelectedStorage {
                name: partition_name,
                device,
            }));
        }
    }
    Ok(None)
}

/// 把选中的设备挂到 `/mnt/sdcard`（创建目录骨架），并记录全局已挂载状态。
///
/// 打印验收日志 `CONTEST02` / `CONTEST03`。真正的目录安装由调用方在
/// 返回后完成（见 `main::install_sdcard_contents`）。
pub fn mount_selected(selected: &SelectedStorage) -> Result<(), StorageError> {
    if !device_is_ext4(&selected.device)? {
        return Err(StorageError::NotExt4);
    }
    // 结构校验：能真正打开 ext4（超级块/块组/特性合法）才置位
    // `contest_storage_mounted`。只有魔数、结构损坏的镜像不得挂载。
    if crate::ext4::Ext4FileSystem::open(Arc::clone(&selected.device)).is_err() {
        return Err(StorageError::NotExt4);
    }
    crate::println!("CONTEST02 filesystem=ext4");

    let _ = crate::fs::mkdir("/mnt", 0o755);
    let _ = crate::fs::mkdir("/mnt/sdcard", 0o755);

    *SELECTED.lock() = Some(selected.clone());
    crate::println!("CONTEST03 mounted=/mnt/sdcard");
    Ok(())
}

/// 竞赛存储是否已挂载。
pub fn contest_storage_mounted() -> bool {
    SELECTED.lock().is_some()
}

/// 返回已挂载的竞赛存储设备（供后续安装逻辑复用同一 `Arc`）。
pub fn contest_storage_device() -> Option<Arc<dyn BlockDevice>> {
    SELECTED.lock().as_ref().map(SelectedStorage::device)
}

/// 返回已挂载的竞赛存储注册表设备名。
pub fn contest_storage_name() -> Option<String> {
    SELECTED
        .lock()
        .as_ref()
        .map(|selected| selected.name.clone())
}

/// 已挂载竞赛存储的 `/dev/<name>` 源路径，供 `fs::mount_ext4_overlay` /
/// `fs::install_ext4_path` 直接作 `source` 参数（fs 层按注册表名解析，
/// 不经 VFS `/dev` 树）。未挂载时返回 `None`。
pub fn contest_source_path() -> Option<String> {
    contest_storage_name().map(|name| alloc::format!("/dev/{}", name))
}

/// 读取设备头部 ext4 超级块魔数（偏移 1024 + 56）。
fn device_is_ext4(device: &Arc<dyn BlockDevice>) -> Result<bool, StorageError> {
    let mut magic = [0_u8; 2];
    let read = block::read_at(
        device,
        EXT4_SUPER_OFFSET + EXT4_SUPER_MAGIC_OFFSET,
        &mut magic,
    )?;
    if read != magic.len() {
        return Ok(false);
    }
    Ok(u16::from_le_bytes(magic) == EXT4_SUPER_MAGIC)
}

#[cfg(debug_assertions)]
pub fn verify() {
    // 1) bootargs 解析。
    assert_eq!(config_from_bootargs(None).device_name, None);
    assert_eq!(
        config_from_bootargs(Some("console=ttyS0 root=/dev/sda1")).device_name,
        None,
    );
    assert_eq!(
        config_from_bootargs(Some("console=ttyS0 sudoos.contest.dev=mmcblk1 maxcpus=1"))
            .device_name
            .as_deref(),
        Some("mmcblk1"),
    );
    assert_eq!(
        config_from_bootargs(Some("sudoos.contest.dev=")).device_name,
        None,
        "empty contest.dev value must be ignored",
    );

    // required 解析（CodePlan §8）：sudoos.contest.required=1/true。
    assert_eq!(
        config_from_bootargs(Some("sudoos.contest.dev=sda sudoos.contest.required=1")).required,
        true,
        "required=1 must set the required flag",
    );
    assert_eq!(
        config_from_bootargs(Some("sudoos.contest.dev=sda sudoos.contest.required=0")).required,
        false,
        "required=0 must leave required unset",
    );
    assert_eq!(
        config_from_bootargs(Some("sudoos.contest.required=true console=ttyS0")).required,
        true,
        "required=true must set the required flag",
    );
    assert_eq!(config_from_bootargs(Some("console=ttyS0")).required, false);
    assert_eq!(config_from_bootargs(None).required, false);

    // 2) 指定存在设备（ext4）。
    let ext4 = make_ext4_device();
    let candidates = vec![(
        String::from("contest-ext4"),
        Arc::clone(&ext4) as Arc<dyn BlockDevice>,
    )];
    let selected = select_from_candidates(
        &ContestStorageConfig {
            device_name: Some(String::from("contest-ext4")),
            ..Default::default()
        },
        &candidates,
    )
    .expect("explicit device selection should succeed")
    .expect("explicit ext4 device should be selected");
    assert_eq!(selected.name(), "contest-ext4");

    // 3) 指定不存在设备 → 安全跳过，不 panic。
    let missing = select_from_candidates(
        &ContestStorageConfig {
            device_name: Some(String::from("ghost-disk")),
            ..Default::default()
        },
        &candidates,
    )
    .expect("missing device should not error");
    assert!(missing.is_none());

    // 4) 自动扫描：第一个 ext4 设备。
    let not_ext4 = Arc::new(MemoryBlockDevice::new(512, 16).expect("non-ext4 fixture"));
    let scanned_candidates = vec![
        (
            String::from("disk-blank"),
            not_ext4.clone() as Arc<dyn BlockDevice>,
        ),
        (
            String::from("contest-ext4"),
            Arc::clone(&ext4) as Arc<dyn BlockDevice>,
        ),
    ];
    let scanned = select_from_candidates(&ContestStorageConfig::default(), &scanned_candidates)
        .expect("auto-scan should succeed")
        .expect("auto-scan should pick the ext4 device");
    assert_eq!(scanned.name(), "contest-ext4");

    // 5) 注册表重复设备名必须被拒绝。
    let contest_reg = Arc::clone(&ext4) as Arc<dyn BlockDevice>;
    assert_eq!(
        block::register_device("contest-reg", contest_reg.clone()),
        Ok(())
    );
    assert_eq!(
        block::register_device("contest-reg", contest_reg),
        Err(BlockError::InvalidArgument),
        "duplicate device name must be rejected",
    );
    block::unregister_device("contest-reg").expect("unregister duplicate probe");

    // 6) 非 ext4 设备不得被显式选中。
    let rejected = select_from_candidates(
        &ContestStorageConfig {
            device_name: Some(String::from("disk-blank")),
            ..Default::default()
        },
        &scanned_candidates,
    );
    assert_eq!(rejected.err(), Some(StorageError::NotExt4));

    // 7) 无任何候选（自动扫描）→ Ok(None)。
    let empty = select_from_candidates(&ContestStorageConfig::default(), &[]);
    assert!(empty.expect("empty scan should not error").is_none());

    // 8) 显式指定整盘非 ext4 的 GPT 盘 → 自动降级到其 ext4 分区
    //    （`select_device` 经注册表 + 分区索引的完整路径）。
    let gpt_disk: Arc<dyn BlockDevice> =
        Arc::new(MemoryBlockDevice::new(512, 64).expect("gpt fixture disk"));
    crate::partition::install_gpt_fixture(&gpt_disk, &[(34, 63, false)]);
    // 分区 1 起始 LBA 34 → 分区内 ext4 超级块偏移 1024+56 落在父盘块 36。
    let mut super_sector = [0_u8; 512];
    super_sector[56..58].copy_from_slice(&0xef53_u16.to_le_bytes());
    gpt_disk
        .write_block(36, &super_sector)
        .expect("write partition ext4 magic");
    block::register_device("gptdisk", Arc::clone(&gpt_disk)).expect("register gpt disk");
    crate::partition::register_all_partitions();
    let descended = select_device(&ContestStorageConfig {
        device_name: Some(String::from("gptdisk")),
        ..Default::default()
    })
    .expect("partition descend selection")
    .expect("gptdisk1 should be selected");
    assert_eq!(descended.name(), "gptdisk1");
    block::unregister_device("gptdisk1").expect("unregister gpt partition");
    block::unregister_device("gptdisk").expect("unregister gpt disk");

    // 9) mount_selected 仅在能真正打开 ext4 后置位，contest_source_path
    //    反映所选设备的 `/dev/<name>` 路径；复位不泄漏到后续。
    let openable = make_openable_ext4_device();
    let selected = SelectedStorage {
        name: String::from("testdisk"),
        device: Arc::clone(&openable) as Arc<dyn BlockDevice>,
    };
    let prior = SELECTED.lock().clone();
    mount_selected(&selected).expect("mount valid ext4");
    assert_eq!(
        contest_source_path().as_deref(),
        Some("/dev/testdisk"),
        "contest_source_path must reflect the mounted device",
    );
    *SELECTED.lock() = prior;

    crate::println!("C1 contest storage gate:");
    crate::println!("  bootargs parsing      : verified");
    crate::println!("  explicit device       : verified");
    crate::println!("  explicit missing      : verified");
    crate::println!("  auto-scan             : verified");
    crate::println!("  duplicate name        : verified");
    crate::println!("  non-ext4 rejected     : verified");
    crate::println!("  no-device safe skip   : verified");
    crate::println!("  GPT partition descend : verified");
    crate::println!("  mount-validates-ext4  : verified");
    crate::println!("  contest_source_path   : verified");
}

#[cfg(debug_assertions)]
fn make_ext4_device() -> Arc<MemoryBlockDevice> {
    let device = Arc::new(MemoryBlockDevice::new(512, 32).expect("ext4 fixture block device"));
    let mut super_sector = [0_u8; 512];
    super_sector[56..58].copy_from_slice(&0xef53_u16.to_le_bytes());
    device
        .write_block(2, &super_sector)
        .expect("write ext4 super magic");
    device
}

/// 结构合法的 ext4 fixture：除魔数外还填上 `Ext4FileSystem::open` 校验的
/// 字段（块大小/每组块/每组 inode/inode 大小），使 `mount_selected` 的
/// "成功打开 ext4" 门能通过。
#[cfg(debug_assertions)]
fn make_openable_ext4_device() -> Arc<MemoryBlockDevice> {
    let device = Arc::new(MemoryBlockDevice::new(512, 32).expect("ext4 fixture block device"));
    let mut super_sector = [0_u8; 512];
    super_sector[24..28].copy_from_slice(&0_u32.to_le_bytes()); // log_block_size = 0 → 1024
    super_sector[32..36].copy_from_slice(&32768_u32.to_le_bytes()); // blocks_per_group
    super_sector[40..44].copy_from_slice(&8192_u32.to_le_bytes()); // inodes_per_group
    super_sector[56..58].copy_from_slice(&0xef53_u16.to_le_bytes()); // magic
    super_sector[88..90].copy_from_slice(&256_u16.to_le_bytes()); // inode_size
    device
        .write_block(2, &super_sector)
        .expect("write ext4 superblock");
    device
}
