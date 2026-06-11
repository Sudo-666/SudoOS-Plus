#[cfg(feature = "platform-qemu-virt")]
mod qemu_virt;

#[cfg(feature = "platform-qemu-virt")]
pub(crate) use qemu_virt::{boot_context, reserve_early_memory, write_console_byte};

#[cfg(not(feature = "platform-qemu-virt"))]
compile_error!("no LoongArch platform has been selected");
