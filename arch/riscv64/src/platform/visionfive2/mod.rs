mod boot;
mod console;
mod memory;

use core::arch::global_asm;

global_asm!(include_str!("entry.S"));

pub(crate) use boot::boot_context;
pub(crate) use console::{
    HAS_CONSOLE_RX, console_line_status, try_read_console_byte, write_console_byte,
};
pub(crate) use memory::reserve_early_memory;

/// VisionFive 2 (JH7110) 只启动 4 个 U74:hart 1..=4。
///
/// hart 0 是 S7 monitor core,即使被错误 DTB 标成 `okay` 也必须排除。
pub(crate) fn hardware_cpu_is_supported(hardware_id: usize) -> bool {
    (1..=4).contains(&hardware_id)
}
