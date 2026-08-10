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

