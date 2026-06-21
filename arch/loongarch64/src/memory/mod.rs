use myos_mm::{MemoryMap, MemoryMapError};
pub mod dmw;
pub mod layout;
pub mod paging;
pub mod phys_access;

/// 排除当前 LoongArch 平台在启动阶段占用的物理内存。
pub fn reserve_early_platform_memory<const CAPACITY: usize>(
    map: &mut MemoryMap<CAPACITY>,
) -> Result<(), MemoryMapError> {
    crate::platform::reserve_early_memory(map)
}
