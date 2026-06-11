use myos_mm::{MemoryMap, MemoryMapError, PhysAddr, PhysRange};

/// QEMU LoongArch direct boot 的启动参数区域。
///
/// QEMU 会在该区域放置：
///
/// - command line；
/// - EFI-style system table；
/// - configuration table；
/// - boot memory map；
/// - initrd descriptor。
///
/// FDT 位于紧随其后的 0x0010_0000。
const BOOT_INFO_BASE: usize = 0;
const BOOT_INFO_SIZE: usize = 1024 * 1024;

pub(crate) fn reserve_early_memory<const CAPACITY: usize>(
    map: &mut MemoryMap<CAPACITY>,
) -> Result<(), MemoryMapError> {
    let boot_info = PhysRange::from_start_size(PhysAddr::new(BOOT_INFO_BASE), BOOT_INFO_SIZE)
        .expect("fixed QEMU boot-info range must be valid");

    map.reserve(boot_info)
}
