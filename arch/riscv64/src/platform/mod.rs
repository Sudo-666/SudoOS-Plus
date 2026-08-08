// 平台选择。
//
// arch-riscv64 的 default feature 会自带 platform-qemu-virt。
// 当 kernel 通过依赖透传显式启用 platform-visionfive2 时,两个 feature
// 会同时出现,因此这里用 not(feature = "platform-visionfive2") 让
// visionfive2 优先,避免"多平台冲突"的硬错误破坏 `cargo build --target
// riscv64` 这类不带平台参数的既有用法。
#[cfg(feature = "platform-visionfive2")]
mod visionfive2;

#[cfg(all(feature = "platform-qemu-virt", not(feature = "platform-visionfive2")))]
mod qemu_virt;

#[cfg(feature = "platform-visionfive2")]
pub(crate) use visionfive2::{boot_context, reserve_early_memory, write_console_byte};

#[cfg(all(feature = "platform-qemu-virt", not(feature = "platform-visionfive2")))]
pub(crate) use qemu_virt::{boot_context, reserve_early_memory, write_console_byte};

#[cfg(not(any(feature = "platform-qemu-virt", feature = "platform-visionfive2")))]
compile_error!("Please select a RISC-V platform feature (e.g. platform-qemu-virt)");
