//! EHCI DMA uncached 区域。
//!
//! 所有 EHCI 描述符（Frame List / QH / qTD）与数据 bounce 缓冲都从
//! `.nocache_ram` 保留的 uncached 物理区（`__nocache_dyn_start ..
//! __nocache_ram_end`，见 arch/loongarch64/platform/ls2k1000/linker.ld）
//! 固定切块。CPU 只经 uncached `0x8000...` 直接映射窗口访问，控制器经
//! 低 32 位物理地址访问；区域内**从不建立 cached 别名**，从类型上杜绝
//! 双窗口缓存一致性问题——这正是 M2.11 / M2.15 / M2.16 一连串 bug 的根因
//! （同一 QH 被 cached `0x9000...` 写入、uncached `0x8000...` 读回，软件
//! 字段不可见）。
//!
//! 与 C/CherryUSB 路径的取舍：本驱动不复用 buddy 动态页做 DMA——回收页带
//! 历史脏缓存行，而本机没有可用的 `cache` 刷写指令（binutils 无法汇编，
//! 见 ADR-001），uncached 写入会被脏行回写覆盖。保留的 uncached 区域在
//! 生命周期内只被 uncached 访问，结构性安全。

use core::ptr;

use crate::usb::error::UsbError;

// `.nocache_ram` 动态池边界符号（ls2k1000 linker.ld 定义，VMA 已是
// uncached `0x8000...` 窗口）。
unsafe extern "C" {
    static __nocache_dyn_start: u8;
    static __nocache_ram_end: u8;
}

/// 低 48 位物理地址掩码：cached/uncached 直接映射窗口都是 `BASE | phys`。
const PHYS_MASK: usize = 0x0fff_ffff_ffff_ffff;
/// uncached 直接映射窗口高 16 位（`0x8000_0000_0000_0000`）。
const UNCACHED_WINDOW: usize = 0x8000_0000_0000_0000;

/// Frame List 表项数（EHCI 1024 × 4B = 4 KiB，页对齐）。
pub const FRAME_LIST_ENTRIES: usize = 1024;
/// 描述符池大小：固定异步队列（4 个 QH）+ qTD 池，32B 对齐。
pub const DESCRIPTOR_POOL_SIZE: usize = 4096;
/// bounce 缓冲大小：单块 512B 读 + 余量（CBW/CSW/SCSI 数据）。
pub const BOUNCE_SIZE: usize = 4096;

fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (value + align - 1) & !(align - 1)
}

/// 动态池边界（uncached 虚拟地址）。
fn pool_bounds() -> (usize, usize) {
    // `addr_of!` 只取地址不解引用，是安全操作；符号由链接脚本定义。
    let start = ptr::addr_of!(__nocache_dyn_start) as usize;
    let end = ptr::addr_of!(__nocache_ram_end) as usize;
    (start, end)
}

/// 一块 uncached DMA 内存。
///
/// 同时持有 CPU 侧 uncached 虚拟地址与控制器侧低 32 位物理地址。不暴露
/// cached 指针，保证访问只走一条路径。长度由分配时的常量保证，不记录在
/// 结构内（调用方对偏移的 bounds 负责，见 volatile 方法的 Safety 文档）。
#[derive(Debug)]
pub struct DmaRegion {
    /// uncached CPU 虚拟地址（`0x8000...`）。
    base: usize,
    /// 控制器 32 位 DMA 物理地址（`base & PHYS_MASK`，< 4 GiB）。
    physical: u32,
}

impl DmaRegion {
    /// 从 bump 游标切一块 `length` 字节、按 `align` 对齐的 uncached 区。
    ///
    /// 只在驱动初始化（单线程）时调用，无锁。`cursor` 是该区之后
    /// 下一次分配的位置。
    fn carve(cursor: &mut usize, length: usize, align: usize) -> Result<Self, UsbError> {
        let start = align_up(*cursor, align);
        let end = pool_bounds().1;
        let stop = start.checked_add(length).ok_or(UsbError::OutOfMemory)?;
        if stop > end {
            return Err(UsbError::OutOfMemory);
        }
        let physical = start & PHYS_MASK;
        if physical > u32::MAX as usize {
            return Err(UsbError::OutOfMemory);
        }
        *cursor = stop;
        Ok(Self {
            base: start,
            physical: physical as u32,
        })
    }

    /// 控制器侧低 32 位物理地址。
    pub const fn physical(&self) -> u32 {
        self.physical
    }

    /// uncached CPU 虚拟地址。
    pub const fn as_usize(&self) -> usize {
        self.base
    }

    /// 经 uncached 窗口以 `T` 类型读 `offset` 处。
    ///
    /// # Safety
    /// - 调用方保证 `offset + size_of::<T>()` 落在本区域分配范围内
    /// - `(self.base + offset)` 满足 `align_of::<T>()` 对齐
    pub unsafe fn read_volatile<T: Copy>(&self, offset: usize) -> T {
        // SAFETY: 调用方保证 bounds 与对齐（见函数文档）。
        unsafe { ptr::read_volatile((self.base + offset) as *const T) }
    }

    /// 经 uncached 窗口以 `T` 类型写 `offset` 处。
    ///
    /// # Safety
    /// 同 [`Self::read_volatile`]。
    pub unsafe fn write_volatile<T>(&self, offset: usize, value: T) {
        // SAFETY: 同 `read_volatile`。
        unsafe { ptr::write_volatile((self.base + offset) as *mut T, value) };
    }
}

/// EHCI 固定 DMA 池：Frame List + 描述符池 + bounce。
///
/// 一次切好、驱动生命周期内常驻（不释放）。控制器/枚举/MSC 共享这三个
/// 区域，保证所有描述符与数据缓冲都落在 uncached 窗口。
#[derive(Debug)]
pub struct DmaPool {
    frame_list: DmaRegion,
    descriptors: DmaRegion,
    bounce: DmaRegion,
}

impl DmaPool {
    /// 从 `.nocache_ram` 动态池切出三个区域。
    pub fn new() -> Result<Self, UsbError> {
        let mut cursor = pool_bounds().0;
        let frame_list = DmaRegion::carve(&mut cursor, FRAME_LIST_ENTRIES * 4, 4096)?;
        let descriptors = DmaRegion::carve(&mut cursor, DESCRIPTOR_POOL_SIZE, 32)?;
        let bounce = DmaRegion::carve(&mut cursor, BOUNCE_SIZE, 512)?;
        Ok(Self {
            frame_list,
            descriptors,
            bounce,
        })
    }
}

/// RUSB-DMA 门禁自检：uncached 别名唯一性 + <4 GiB + uncached 零回环。
///
/// 打印 `RUSB-DMA va=... pa=... below4g=1 zero-roundtrip=0 alias=unique
/// PASS`。任何一项不满足返回 `UsbError`（调用方只打日志，不 panic）。
pub fn dma_gate() -> Result<(), UsbError> {
    let pool = DmaPool::new()?;
    let regions = [
        (pool.frame_list.as_usize(), pool.frame_list.physical()),
        (pool.descriptors.as_usize(), pool.descriptors.physical()),
        (pool.bounce.as_usize(), pool.bounce.physical()),
    ];

    let mut below4g = true;
    let mut alias_unique = true;
    for (i, &(va, pa)) in regions.iter().enumerate() {
        // 物理地址从 uncached VA 剥离（usize 比较），EHCI 32 位 DMA 必须
        // < 4 GiB（carve 已校验，这里独立复核）。
        if (va & PHYS_MASK) >= 0x1_0000_0000 {
            below4g = false;
        }
        // uncached 窗口校验：高 16 位必须是 0x8000（否则混进了 cached
        // 0x9000... 别名，就是 M2.16 的故障形态）。
        if va & !PHYS_MASK != UNCACHED_WINDOW {
            alias_unique = false;
        }
        for (j, &(_, other_pa)) in regions.iter().enumerate() {
            if i != j && pa == other_pa {
                alias_unique = false;
            }
        }
    }

    // uncached 零回环：写入模式立即读回一致（无 cached 层介入）。
    // SAFETY: bounce 长度 4096 >= 4，offset 0 满足 u32 对齐。
    unsafe {
        pool.bounce.write_volatile(0usize, 0xdeadu32);
    }
    // SAFETY: 同写。
    let roundtrip: u32 = unsafe { pool.bounce.read_volatile(0) };
    let roundtrip_ok = roundtrip == 0xdead;

    if below4g && roundtrip_ok && alias_unique {
        crate::println!(
            "RUSB-DMA va={:016x} pa={:08x} below4g=1 zero-roundtrip=0 alias=unique PASS",
            regions[0].0,
            regions[0].1,
        );
        Ok(())
    } else {
        crate::println!(
            "RUSB-DMA va={:016x} pa={:08x} below4g={} zero-roundtrip={} alias={} FAIL",
            regions[0].0,
            regions[0].1,
            if below4g { 1 } else { 0 },
            if roundtrip_ok { 0 } else { 1 },
            if alias_unique { "unique" } else { "dup" },
        );
        Err(UsbError::InvalidState)
    }
}
