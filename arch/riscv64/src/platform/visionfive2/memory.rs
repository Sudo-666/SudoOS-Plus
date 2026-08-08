use myos_mm::{MemoryMap, MemoryMapError, PhysAddr, PhysRange};

/// 保留 OpenSBI 固件区。
///
/// VisionFive 2 固件链: ZSBL -> SPL(M-mode) -> OpenSBI(M-mode, FW_TEXT_START
/// = 0x4000_0000) -> U-Boot proper(S-mode)。内核镜像加载在 0x4020_0000,
/// OpenSBI 常驻 [0x4000_0000, 0x4020_0000) 提供 SBI 运行时服务,不能释放给
/// 页分配器。这里显式保留,即使 FDT 的 memory reservation 缺失也安全。
pub(crate) fn reserve_early_memory<const CAPACITY: usize>(
    map: &mut MemoryMap<CAPACITY>,
) -> Result<(), MemoryMapError> {
    let opensbi = PhysRange::from_start_size(PhysAddr::new(0x4000_0000), 0x0020_0000)
        .expect("OpenSBI firmware range must be valid");

    map.reserve(opensbi)
}
