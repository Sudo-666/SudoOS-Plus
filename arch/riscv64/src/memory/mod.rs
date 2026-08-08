use myos_mm::{MemoryMap, MemoryMapError};
pub mod layout;
pub mod paging;
pub mod phys_access;

/// 排除 RISC-V 平台额外占用的启动期内存,由所选平台提供。
///
/// - qemu_virt: OpenSBI/QEMU 占用已由 FDT memory reservation 描述,不额外保留
/// - visionfive2: 显式保留 OpenSBI 固件区 [0x4000_0000, 0x4020_0000)
pub fn reserve_early_platform_memory<const CAPACITY: usize>(
    map: &mut MemoryMap<CAPACITY>,
) -> Result<(), MemoryMapError> {
    crate::platform::reserve_early_memory(map)
}
