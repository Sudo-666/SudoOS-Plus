use myos_mm::{
    PhysAddr, VirtAddr, VirtRange, VirtualLayoutError, VirtualRegion, require_address_in_region,
    validate_regions,
};

pub const VA_BITS: usize = 39;

pub const USER_RANGE: VirtRange =
    VirtRange::from_bounds(0x0000_0000_0000_0000, 0x0000_0040_0000_0000);

pub const KERNEL_CANONICAL_START: VirtAddr = VirtAddr::new(0xffff_ffc0_0000_0000);

pub const FIXMAP: VirtRange = VirtRange::from_bounds(0xffff_ffc4_fea0_0000, 0xffff_ffc4_ff00_0000);

pub const PCI_IO: VirtRange = VirtRange::from_bounds(0xffff_ffc4_ff00_0000, 0xffff_ffc5_0000_0000);

pub const VMEMMAP: VirtRange = VirtRange::from_bounds(0xffff_ffc5_0000_0000, 0xffff_ffc6_0000_0000);

pub const VMALLOC: VirtRange = VirtRange::from_bounds(0xffff_ffc6_0000_0000, 0xffff_ffd6_0000_0000);

/// 物理地址零映射到 `DIRECT_MAP.start()`。
///
/// 当前 Sv39 布局提供 128 GiB direct map。
pub const DIRECT_MAP: VirtRange =
    VirtRange::from_bounds(0xffff_ffd6_0000_0000, 0xffff_fff6_0000_0000);

pub const MODULES: VirtRange = VirtRange::from_bounds(0xffff_ffff_0000_0000, 0xffff_ffff_8000_0000);

/// 最后一页保留为不可访问保护页。
pub const KERNEL_IMAGE: VirtRange =
    VirtRange::from_bounds(0xffff_ffff_8000_0000, 0xffff_ffff_ffff_f000);

pub const KERNEL_REGIONS: &[VirtualRegion] = &[
    VirtualRegion::new("fixmap", FIXMAP),
    VirtualRegion::new("pci-io", PCI_IO),
    VirtualRegion::new("vmemmap", VMEMMAP),
    VirtualRegion::new("vmalloc", VMALLOC),
    VirtualRegion::new("direct-map", DIRECT_MAP),
    VirtualRegion::new("modules", MODULES),
    VirtualRegion::new("kernel-image", KERNEL_IMAGE),
];

pub const fn is_canonical(address: VirtAddr) -> bool {
    address.get() < USER_RANGE.end().get() || address.get() >= KERNEL_CANONICAL_START.get()
}

pub const fn is_user_address(address: VirtAddr) -> bool {
    USER_RANGE.contains(address)
}

pub const fn is_kernel_address(address: VirtAddr) -> bool {
    address.get() >= KERNEL_CANONICAL_START.get()
}

pub fn validate() -> Result<(), VirtualLayoutError> {
    validate_regions(KERNEL_REGIONS, is_kernel_address)?;

    require_address_in_region(
        "kernel link address",
        KERNEL_LINK_BASE,
        "kernel-image",
        KERNEL_IMAGE,
    )
}

pub fn phys_to_direct(physical: PhysAddr) -> Option<VirtAddr> {
    if physical.get() >= DIRECT_MAP.size() {
        return None;
    }

    DIRECT_MAP.start().checked_add(physical.get())
}

pub fn direct_to_phys(virtual_address: VirtAddr) -> Option<PhysAddr> {
    if !DIRECT_MAP.contains(virtual_address) {
        return None;
    }

    Some(PhysAddr::new(
        virtual_address.get() - DIRECT_MAP.start().get(),
    ))
}

pub fn kernel_image_virtual_address(physical: PhysAddr) -> Option<VirtAddr> {
    let offset = physical.get().checked_sub(KERNEL_PHYS_BASE.get())?;

    let virtual_address = KERNEL_LINK_BASE.checked_add(offset)?;

    if !KERNEL_IMAGE.contains(virtual_address) {
        return None;
    }

    Some(virtual_address)
}

pub fn kernel_image_physical_address(virtual_address: VirtAddr) -> Option<PhysAddr> {
    if !KERNEL_IMAGE.contains(virtual_address) {
        return None;
    }

    let offset = virtual_address.get().checked_sub(KERNEL_LINK_BASE.get())?;

    KERNEL_PHYS_BASE.checked_add(offset)
}

// OpenSBI 跳转进入的低物理启动地址。
pub const BOOT_PHYS_BASE: PhysAddr = PhysAddr::new(0x8020_0000);

/// 正式高半内核的物理加载地址。
///
/// 保持 2 MiB 对齐，便于启动临时页表使用 2 MiB 叶项。
pub const KERNEL_PHYS_BASE: PhysAddr = PhysAddr::new(0x8040_0000);

pub const KERNEL_LINK_BASE: VirtAddr = VirtAddr::new(0xffff_ffff_8000_0000);
