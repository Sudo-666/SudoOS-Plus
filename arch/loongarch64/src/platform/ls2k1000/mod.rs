use core::arch::global_asm;

// 引入 LS2K1000 专用的 entry.S
global_asm!(include_str!("entry.S"));
global_asm!(include_str!("secondary.S"));

mod boot;
mod console;
mod memory;

pub(crate) use boot::boot_context;
pub(crate) use console::{
    HAS_CONSOLE_RX, console_line_status, try_read_console_byte, write_console_byte,
};
pub(crate) use memory::reserve_early_memory;

/// LS2K1000 接受 FDT 中所有 available 的 CPU(硬件 ID 合法性由
/// start_secondary 的 HARDWARE_CPU_ID_LIMIT 二次校验)。
pub(crate) fn hardware_cpu_is_supported(_hardware_id: usize) -> bool {
    true
}
