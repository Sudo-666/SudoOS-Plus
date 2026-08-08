use myos_mm::{
    PhysAddr, VirtAddr, VirtRange, VirtualLayoutError, VirtualRegion, require_address_in_region,
    validate_regions,
};

/// LoongArch DMW 当前允许直接表示的物理地址宽度。
pub const DMW_PHYS_BITS: usize = 48;

pub const DMW_PHYS_MASK: usize = (1_usize << DMW_PHYS_BITS) - 1;

/// LoongArch 架构地址段的起点。
///
/// 这些是体系结构分段，不代表页表实际支持的地址宽度。
pub const XUVRANGE_START: usize = 0x0000_0000_0000_0000;

pub const XSPRANGE_START: usize = 0x4000_0000_0000_0000;

pub const XKPRANGE_START: usize = 0x8000_0000_0000_0000;

pub const XKVRANGE_START: usize = 0xc000_0000_0000_0000;

/// 当前 MyOS 选择 4 KiB、四级页表。
///
/// QEMU virt (LA464) 提供 48 位虚拟地址；LS2K1000 的 LA264 核只有
/// 40 位（CPUCFG0 VALEN=40）。LA264 上内核页表区必须位于 bit39
/// 符号扩展地址（bits[63:40] = 0xFF），因此下列区域按平台区分。
#[cfg(not(feature = "platform-ls2k1000"))]
pub const PAGE_TABLE_VA_BITS: usize = 48;

#[cfg(feature = "platform-ls2k1000")]
pub const PAGE_TABLE_VA_BITS: usize = 40;

/// 当前编译配置允许的最大用户虚拟地址范围。
///
/// 之后读取 CPUCFG1.VABITS 后，实际用户范围还应取：
///
/// min(PAGE_TABLE_VA_BITS, cpu_vabits)
#[cfg(not(feature = "platform-ls2k1000"))]
pub const USER_RANGE: VirtRange =
    VirtRange::from_bounds(0x0000_0000_0000_0000, 0x0001_0000_0000_0000);

/// LS2K1000 (LA264, VALEN=40)：合法用户地址的 bit39 必须为 0，即 [0, 2^39)。
#[cfg(feature = "platform-ls2k1000")]
pub const USER_RANGE: VirtRange =
    VirtRange::from_bounds(0x0000_0000_0000_0000, 0x0000_0080_0000_0000);

/// 强序 uncached 直接映射。
pub const UNCACHED_DIRECT_MAP: VirtRange =
    VirtRange::from_bounds(0x8000_0000_0000_0000, 0x8001_0000_0000_0000);

/// coherent cached 直接映射。
pub const CACHED_DIRECT_MAP: VirtRange =
    VirtRange::from_bounds(0x9000_0000_0000_0000, 0x9001_0000_0000_0000);

/// 页表映射的动态内核虚拟地址区域。
#[cfg(not(feature = "platform-ls2k1000"))]
pub const VMALLOC: VirtRange = VirtRange::from_bounds(0xffff_8000_0000_0000, 0xffff_c000_0000_0000);

/// LS2K1000：内核页表区位于 bit39 符号扩展的 40 位空间
/// (bits[63:40] = 0xFF)。其顶层 PGD 索引（bits[47:39] = 511）与 QEMU
/// 平台的 48 位高半区一致，页表几何无需改变。
#[cfg(feature = "platform-ls2k1000")]
pub const VMALLOC: VirtRange = VirtRange::from_bounds(0xffff_ff80_0000_0000, 0xffff_ffc0_0000_0000);

#[cfg(not(feature = "platform-ls2k1000"))]
pub const MODULES: VirtRange = VirtRange::from_bounds(0xffff_c000_0000_0000, 0xffff_c100_0000_0000);

#[cfg(feature = "platform-ls2k1000")]
pub const MODULES: VirtRange = VirtRange::from_bounds(0xffff_ffc0_0000_0000, 0xffff_ffc1_0000_0000);

/// FIXMAP 地址 0xffff_fffe_0000_0000 在 40 位符号扩展范围内同样合法，
/// 两个平台保持一致。
pub const FIXMAP: VirtRange = VirtRange::from_bounds(0xffff_fffe_0000_0000, 0xffff_ffff_0000_0000);

/// 最终运行地址使用 cached DMW 高地址别名。
///
/// QEMU virt 把低地址启动入口加载到 0x0020_0000；
/// LS2K1000 的 1GB DDR 物理基址是 0x9000_0000，
/// U-Boot 把内核镜像加载到 DDR 基址并跳转到那里。
#[cfg(feature = "platform-qemu-virt")]
pub const BOOT_PHYS_BASE: PhysAddr = PhysAddr::new(0x0020_0000);

#[cfg(feature = "platform-ls2k1000")]
pub const BOOT_PHYS_BASE: PhysAddr = PhysAddr::new(0x9000_0000);

#[cfg(feature = "platform-qemu-virt")]
pub const BOOT_VIRT_BASE: VirtAddr = VirtAddr::new(0x9000_0000_0020_0000);

#[cfg(feature = "platform-ls2k1000")]
pub const BOOT_VIRT_BASE: VirtAddr = VirtAddr::new(0x9000_0000_9000_0000);

/// 正式内核保持 2 MiB 对齐，方便以后使用大页映射。
#[cfg(feature = "platform-qemu-virt")]
pub const KERNEL_PHYS_BASE: PhysAddr = PhysAddr::new(0x0040_0000);

#[cfg(feature = "platform-ls2k1000")]
pub const KERNEL_PHYS_BASE: PhysAddr = PhysAddr::new(0x9020_0000);

#[cfg(feature = "platform-qemu-virt")]
pub const KERNEL_LINK_BASE: VirtAddr = VirtAddr::new(0x9000_0000_0040_0000);

#[cfg(feature = "platform-ls2k1000")]
pub const KERNEL_LINK_BASE: VirtAddr = VirtAddr::new(0x9000_0000_9020_0000);

pub const KERNEL_REGIONS: &[VirtualRegion] = &[
    VirtualRegion::new("uncached-direct-map", UNCACHED_DIRECT_MAP),
    VirtualRegion::new("cached-direct-map", CACHED_DIRECT_MAP),
    VirtualRegion::new("vmalloc", VMALLOC),
    VirtualRegion::new("modules", MODULES),
    VirtualRegion::new("fixmap", FIXMAP),
];

pub const fn is_user_address(address: VirtAddr) -> bool {
    USER_RANGE.contains(address)
}

/// 当前 MyOS 只使用 XKPRANGE 与 XKVRANGE 作为内核地址。
pub const fn is_kernel_address(address: VirtAddr) -> bool {
    address.get() >= XKPRANGE_START
}

pub fn validate() -> Result<(), VirtualLayoutError> {
    validate_regions(KERNEL_REGIONS, is_kernel_address)?;

    require_address_in_region(
        "kernel link address",
        KERNEL_LINK_BASE,
        "cached-direct-map",
        CACHED_DIRECT_MAP,
    )
}

pub fn phys_to_cached(physical: PhysAddr) -> Option<VirtAddr> {
    if physical.get() & !DMW_PHYS_MASK != 0 {
        return None;
    }

    Some(VirtAddr::new(
        CACHED_DIRECT_MAP.start().get() | physical.get(),
    ))
}

pub fn cached_to_phys(virtual_address: VirtAddr) -> Option<PhysAddr> {
    if !CACHED_DIRECT_MAP.contains(virtual_address) {
        return None;
    }

    Some(PhysAddr::new(virtual_address.get() & DMW_PHYS_MASK))
}

pub fn phys_to_uncached(physical: PhysAddr) -> Option<VirtAddr> {
    if physical.get() & !DMW_PHYS_MASK != 0 {
        return None;
    }

    Some(VirtAddr::new(
        UNCACHED_DIRECT_MAP.start().get() | physical.get(),
    ))
}

pub fn uncached_to_phys(virtual_address: VirtAddr) -> Option<PhysAddr> {
    if !UNCACHED_DIRECT_MAP.contains(virtual_address) {
        return None;
    }

    Some(PhysAddr::new(virtual_address.get() & DMW_PHYS_MASK))
}
