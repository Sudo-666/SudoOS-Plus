use core::sync::atomic::{AtomicBool, Ordering};

/// 两个 RISC-V 平台的早期 UART 物理基址相同 (0x1000_0000)。
///
/// - qemu_virt: ns16550a @ 0x1000_0000
/// - visionfive2: JH7110 UART0 @ 0x1000_0000
pub const MMIO_BASE: usize = 0x1000_0000;
pub const MMIO_SIZE: usize = 0x1000;

static RUNTIME_MAPPING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Publishes the high-half UART alias after the final Sv39 root is active.
pub fn activate_runtime_mapping() {
    assert!(
        crate::memory::paging::translation_is_enabled(),
        "RISC-V UART runtime mapping requires Sv39",
    );
    RUNTIME_MAPPING_ACTIVE.store(true, Ordering::Release);
}

/// Returns the UART virtual base valid in the currently published boot phase.
pub fn virtual_base() -> usize {
    if RUNTIME_MAPPING_ACTIVE.load(Ordering::Acquire) {
        crate::memory::layout::EARLY_UART_FIXMAP.get()
    } else {
        MMIO_BASE
    }
}

/// 启动阶段输出一个字节,由所选择的平台提供寄存器布局。
pub fn write_byte(byte: u8) {
    crate::platform::write_console_byte(byte);
}
