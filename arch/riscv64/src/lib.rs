#![no_std]

#[cfg(not(target_arch = "riscv64"))]
compile_error!("arch-riscv64 can only be built for riscv64");

use core::arch::global_asm;

global_asm!(include_str!("asm/entry.S"));
global_asm!(include_str!("trap/entry.S"));

pub const ARCH_NAME: &str = "riscv64";

pub mod boot;
pub mod cpu;
pub mod early_console;
pub mod interrupt;
pub mod memory;
pub mod trap;
