use myos_mm::{MemoryMap, MemoryMapError};

/// QEMU virt 的 OpenSBI/固件占用由 FDT memory reservation 描述,
/// 这里不额外硬编码,保持与历史行为一致。
pub(crate) fn reserve_early_memory<const CAPACITY: usize>(
    _map: &mut MemoryMap<CAPACITY>,
) -> Result<(), MemoryMapError> {
    Ok(())
}
