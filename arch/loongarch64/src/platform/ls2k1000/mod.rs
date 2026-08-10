use core::arch::global_asm;

// 引入 LS2K1000 专用的 entry.S
global_asm!(include_str!("entry.S"));
global_asm!(include_str!("secondary.S"));

mod boot;
mod console;
mod memory;

pub(crate) use boot::boot_context;
pub(crate) use console::{try_read_console_byte, write_console_byte};
pub(crate) use memory::reserve_early_memory;