#![feature(unsigned_is_multiple_of)]
#![no_std]

#[cfg(not(target_arch = "riscv64"))]
compile_error!("arch-riscv64 can only be built for riscv64");

use core::arch::global_asm;

// entry.S 已移至各平台目录(platform/qemu_virt/ 或 platform/visionfive2/),
// 由各平台 mod.rs 自行 global_asm!(include_str!("entry.S"))。
// secondary.S 是纯 SBI HSM + 链接符号驱动,与平台无关,保持共享。
global_asm!(include_str!("asm/secondary.S"));
global_asm!(include_str!("trap/entry.S"));
global_asm!(include_str!("task/switch.S"));

pub const ARCH_NAME: &str = "riscv64";

pub mod boot;
pub mod cpu;
pub mod early_console;
pub mod interrupt;
pub mod memory;
mod sbi;
pub mod smp;
pub mod task;
pub mod time;
pub mod trap;

mod platform;
