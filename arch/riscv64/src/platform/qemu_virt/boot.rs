use crate::boot::BootContext;

/// OpenSBI 启动约定:
///
/// - a0: hart ID
/// - a1: FDT 地址
/// - a2: 当前阶段保留
pub(crate) fn boot_context(hart_id: usize, device_tree: usize, reserved: usize) -> BootContext {
    let mut context = BootContext::new([hart_id, device_tree, reserved]);

    if device_tree != 0 {
        context = context.with_device_tree(device_tree);
    }

    context
}
