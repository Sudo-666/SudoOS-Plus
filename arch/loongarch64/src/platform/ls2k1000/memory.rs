// arch/loongarch64/src/platform/ls2k1000/memory.rs

/// 内存偏移量掩码，用于从物理地址获取偏移
pub const PHYS_MASK: usize = 0x0FFF_FFFF_FFFF_FFFF;

/// 一致可缓存的直接映射窗口前缀 (0x9000_0000_0000_0000)[cite: 1]
pub const CACHED_BASE: usize = 0x9000_0000_0000_0000;

/// 强序非缓存的直接映射窗口前缀 (0x8000_0000_0000_0000)[cite: 1]
pub const UNCACHED_BASE: usize = 0x8000_0000_0000_0000;

/// 开发板 RAM 的物理起始地址 (通常为 0x9000_0000)
pub const PHYS_MEMORY_BASE: usize = 0x9000_0000;

/// 开发板板载内存总容量: 2GB DDR3
///
/// 板级 U-Boot 实际报告 "DRAM: 2048 MiB / 2 GiB"
/// (Build: ...-2k1000-dp-2G-factory-7),手册中的 1GB 描述不准确。
/// 注: 引导时内核内存布局以 FDT memory 节点为准,此常量仅作 FDT 缺失兜底。
pub const PHYS_MEMORY_SIZE: usize = 2 * 1024 * 1024 * 1024; // 2 GB

/// 开发板可用物理内存的结束地址
pub const PHYS_MEMORY_END: usize = PHYS_MEMORY_BASE + PHYS_MEMORY_SIZE;

/// 将物理地址转换为内核直接映射的可缓存虚拟地址
#[inline]
pub fn phys_to_virt(paddr: usize) -> usize {
    (paddr & PHYS_MASK) | CACHED_BASE
}

/// 将内核直接映射的虚拟地址转换回物理地址
#[inline]
pub fn virt_to_phys(vaddr: usize) -> usize {
    vaddr & PHYS_MASK
}

/// 保留 U-Boot 启动阶段占用的物理内存区域。
///
/// 2K1000 的 1GB DDR 物理基址是 0x9000_0000，内核镜像加载在 DDR 基址。
/// U-Boot 的启动数据（FDT 等）可能落在低地址 SoC 空间（如 0x0ecce600），
/// 也可能在 DDR 内。这里保留 0x0 开始的 2 MiB 低地址区域，防止万一
/// U-Boot 数据落在该范围时被页分配器复用（对 DDR 范围外的区域，
/// MemoryMap::reserve 是安全的 no-op）。
pub(crate) fn reserve_early_memory<const CAPACITY: usize>(
    map: &mut myos_mm::MemoryMap<CAPACITY>,
) -> Result<(), myos_mm::MemoryMapError> {
    use myos_mm::{PhysAddr, PhysRange};

    let uboot_data = PhysRange::from_start_size(PhysAddr::new(0x0000_0000), 0x0020_0000)
        .expect("U-Boot boot data range must be valid");

    map.reserve(uboot_data)
}