mod boot;
mod console;
mod memory;

use core::arch::global_asm;
// 引入 QEMU 专用的 entry.S
global_asm!(include_str!("entry.S"));
global_asm!(include_str!("secondary.S"));

pub(crate) use boot::boot_context;
pub(crate) use console::{line_status, try_read_console_byte, write_console_byte};
pub(crate) use memory::reserve_early_memory;

/// QEMU virt 接受 FDT 中所有 available 的 CPU(硬件 ID 合法性由
/// start_secondary 的 HARDWARE_CPU_ID_LIMIT 二次校验)。
pub(crate) fn hardware_cpu_is_supported(_hardware_id: usize) -> bool {
    true
}
