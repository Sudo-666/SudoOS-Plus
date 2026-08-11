mod boot;
mod console;
mod memory;

use core::arch::global_asm;

global_asm!(include_str!("entry.S"));

pub(crate) use boot::boot_context;
pub(crate) use console::{
    console_line_status, try_read_console_byte, write_console_byte, HAS_CONSOLE_RX,
};
pub(crate) use memory::reserve_early_memory;

/// QEMU virt 接受 FDT 中所有 available 的 hart。
pub(crate) fn hardware_cpu_is_supported(_hardware_id: usize) -> bool {
    true
}
