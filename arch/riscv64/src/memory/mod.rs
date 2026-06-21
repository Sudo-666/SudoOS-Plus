use myos_mm::{MemoryMap, MemoryMapError};
pub mod layout;
pub mod paging;
pub mod phys_access;

/// 排除 RISC-V 平台额外占用的启动期内存。
///
/// 当前 OpenSBI/QEMU 已通过 FDT memory reservation block
/// 描述固件占用区域，因此这里不额外硬编码 OpenSBI 地址。
///
/// 真实开发板若没有正确填写 FDT reservation，需要在对应
/// 平台实现中增加明确的固件保留范围。
pub fn reserve_early_platform_memory<const CAPACITY: usize>(
    _map: &mut MemoryMap<CAPACITY>,
) -> Result<(), MemoryMapError> {
    Ok(())
}
