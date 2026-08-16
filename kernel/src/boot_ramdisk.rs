//! LS2K1000 固件加载的竞赛镜像 → 只读 `/dev/ram0` 块设备。
//!
//! U-Boot 把竞赛镜像（32 MiB ext4）加载到预留物理内存区域后，本模块把该
//! 区域作为零拷贝只读块设备暴露给存储层（CodePlan C4）。物理访问隔离在
//! [`PhysicalByteSource`] trait 后面，便于用 mock 源做单元测试。

use alloc::sync::Arc;

use myos_mm::PhysAddr;

use crate::{
    block::{self, BlockDevice, BlockError},
};

/// 注册时固定的扇区大小（与 ext4 块设备一致）。
pub const RAMDISK_BLOCK_SIZE: usize = 512;

/// 从物理内存区域拷贝字节。
pub trait PhysicalByteSource: Send + Sync + 'static {
    /// 把 `[offset, offset + output.len())` 拷贝到 `output`。
    /// `offset` 为相对该源基址的字节偏移。
    fn copy_from_physical(&self, offset: usize, output: &mut [u8]) -> Result<(), BlockError>;
}

/// 真实物理内存源：基址 + 直接映射访问。
pub struct PhysicalRamSource {
    base: PhysAddr,
}

impl PhysicalRamSource {
    pub fn new(base: PhysAddr) -> Self {
        Self { base }
    }
}

impl PhysicalByteSource for PhysicalRamSource {
    fn copy_from_physical(&self, offset: usize, output: &mut [u8]) -> Result<(), BlockError> {
        if output.is_empty() {
            return Ok(());
        }
        let address = self
            .base
            .get()
            .checked_add(offset)
            .ok_or(BlockError::AddressOverflow)?;
        let pointer = crate::arch::memory::phys_access::ram_ptr::<u8>(PhysAddr::new(address))
            .map_err(|_| BlockError::AddressOverflow)?;
        // SAFETY: 调用方保证 [base, base+len) 位于内核生命周期内有效且受
        // 保留的 RAM；`offset + output.len()` 不超过该区域长度（由
        // BootRamBlockDevice 边界检查保证）。
        unsafe {
            output.copy_from_slice(core::slice::from_raw_parts(pointer, output.len()));
        }
        Ok(())
    }
}

/// 只读物理内存块设备（零拷贝，不复制镜像到 `Vec`）。
pub struct BootRamBlockDevice<S: PhysicalByteSource> {
    source: S,
    byte_len: usize,
    block_size: usize,
}

impl<S: PhysicalByteSource> BootRamBlockDevice<S> {
    /// 构造只读内存块设备。`byte_len` 必须是 `block_size` 的倍数且非零。
    pub fn new(source: S, byte_len: usize, block_size: usize) -> Result<Self, BlockError> {
        if byte_len == 0 || block_size == 0 || !block_size.is_power_of_two() {
            return Err(BlockError::BadBlockSize);
        }
        if byte_len % block_size != 0 {
            return Err(BlockError::BadBlockSize);
        }
        Ok(Self {
            source,
            byte_len,
            block_size,
        })
    }

    fn block_offset(&self, block: u64) -> Result<usize, BlockError> {
        let offset = usize::try_from(block)
            .ok()
            .and_then(|block| block.checked_mul(self.block_size))
            .ok_or(BlockError::AddressOverflow)?;
        if offset >= self.byte_len {
            return Err(BlockError::OutOfRange);
        }
        Ok(offset)
    }
}

impl<S: PhysicalByteSource> BlockDevice for BootRamBlockDevice<S> {
    fn block_size(&self) -> usize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        (self.byte_len / self.block_size) as u64
    }

    fn read_block(&self, block: u64, output: &mut [u8]) -> Result<(), BlockError> {
        if output.len() != self.block_size {
            return Err(BlockError::BufferTooSmall);
        }
        let offset = self.block_offset(block)?;
        self.source.copy_from_physical(offset, output)
    }

    fn write_block(&self, _block: u64, _input: &[u8]) -> Result<(), BlockError> {
        Err(BlockError::DeviceReadOnly)
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

/// 把固件加载的竞赛镜像注册为 `/dev/ram0`。必须在 `fs::initialize()` 之前
/// 调用（区域已在 buddy allocator 建立前从 free memory 排除，见 C5）。
pub fn register_boot_ramdisk(
    base: PhysAddr,
    byte_len: usize,
    block_size: usize,
) -> Result<(), BlockError> {
    if byte_len == 0 || byte_len % RAMDISK_BLOCK_SIZE != 0 {
        return Err(BlockError::BadBlockSize);
    }
    base.get()
        .checked_add(byte_len)
        .ok_or(BlockError::AddressOverflow)?;
    let device = BootRamBlockDevice::new(PhysicalRamSource::new(base), byte_len, block_size)?;
    block::register_device("ram0", Arc::new(device))
}

/// 返回 `/dev/ram0`（若已注册）。供启动路径在 mount 前查询。
pub fn boot_ramdisk_device() -> Option<Arc<dyn BlockDevice>> {
    block::open_device("ram0")
}

#[cfg(debug_assertions)]
pub fn verify() {
    // 1) 首块、末块读取。
    let data: alloc::vec::Vec<u8> = (0..4 * 512).map(|i| (i % 251) as u8).collect();
    let device = BootRamBlockDevice::new(MockSource { data }, 4 * 512, 512)
        .expect("boot ramdisk construction");
    assert_eq!(device.block_count(), 4);

    let mut first = [0_u8; 512];
    device.read_block(0, &mut first).expect("first block read");
    assert_eq!(&first[..8], &[0, 1, 2, 3, 4, 5, 6, 7]);

    let mut last = [0_u8; 512];
    device.read_block(3, &mut last).expect("last block read");
    assert_eq!(&last[..8], &[36, 37, 38, 39, 40, 41, 42, 43]);

    // 2) 越界读取。
    let mut scratch = [0_u8; 512];
    assert_eq!(
        device.read_block(4, &mut scratch),
        Err(BlockError::OutOfRange),
    );

    // 3) 输出缓冲区大小错误。
    let mut short = [0_u8; 511];
    assert_eq!(
        device.read_block(0, &mut short),
        Err(BlockError::BufferTooSmall),
    );

    // 4) 写入拒绝（只读）。
    let mut input = [0_u8; 512];
    assert_eq!(
        device.write_block(0, &input),
        Err(BlockError::DeviceReadOnly),
    );
    assert!(device.is_read_only());

    // 5) 非 512 对齐长度 / 零长度拒绝。
    assert_eq!(
        BootRamBlockDevice::new(MockSource { data: alloc::vec![0; 1000] }, 1000, 512).err(),
        Some(BlockError::BadBlockSize),
    );
    assert_eq!(
        BootRamBlockDevice::new(MockSource { data: alloc::vec![0; 0] }, 0, 512).err(),
        Some(BlockError::BadBlockSize),
    );

    // 6) 源内地址溢出传播。
    let src = MockSource { data: alloc::vec![0_u8; 512] };
    let mut out = [0_u8; 512];
    assert_eq!(
        src.copy_from_physical(usize::MAX - 10, &mut out),
        Err(BlockError::AddressOverflow),
    );

    // 7) 源内越界传播。
    assert_eq!(
        src.copy_from_physical(0, &mut out),
        Err(BlockError::OutOfRange),
        "offset beyond source length must propagate",
    );

    crate::println!("C4 boot ramdisk gate:");
    crate::println!("  first/last block    : verified");
    crate::println!("  out of bounds       : verified");
    crate::println!("  buffer size         : verified");
    crate::println!("  write rejected      : verified");
    crate::println!("  alignment check     : verified");
    crate::println!("  address overflow    : verified");
    crate::println!("  read-only           : verified");
}

#[cfg(debug_assertions)]
struct MockSource {
    data: alloc::vec::Vec<u8>,
}

#[cfg(debug_assertions)]
impl PhysicalByteSource for MockSource {
    fn copy_from_physical(&self, offset: usize, output: &mut [u8]) -> Result<(), BlockError> {
        let end = offset
            .checked_add(output.len())
            .ok_or(BlockError::AddressOverflow)?;
        if end > self.data.len() {
            return Err(BlockError::OutOfRange);
        }
        output.copy_from_slice(&self.data[offset..end]);
        Ok(())
    }
}
