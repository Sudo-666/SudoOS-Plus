#![no_std]

#[cfg(not(target_arch = "loongarch64"))]
compile_error!("arch-loongarch64 can only be built for loongarch64");

use core::arch::global_asm;

global_asm!(include_str!("asm/entry.S"));
global_asm!(include_str!("memory/paging/refill.S"));
global_asm!(include_str!("trap/entry.S"));

pub const ARCH_NAME: &str = "loongarch64";

pub mod boot;
pub mod cpu;
pub mod early_console;
pub mod interrupt;
pub mod memory;
pub mod trap;

mod platform;
