#[cfg(feature = "platform-qemu-virt")]
mod qemu_virt;

#[cfg(feature = "platform-qemu-virt")]
pub(crate) use qemu_virt::{boot_context, reserve_early_memory, write_console_byte};


// 新增平台架构
#[cfg(feature = "platform-ls2k1000")]
pub mod ls2k1000;
#[cfg(feature = "platform-ls2k1000")]
pub(crate) use ls2k1000::*;

// 防止漏选平台
#[cfg(not(any(feature = "platform-qemu-virt", feature = "platform-ls2k1000")))]
compile_error!("Please select a platform feature (e.g. platform-ls2k1000)");

// 防止多选平台导致冲突
#[cfg(all(feature = "platform-qemu-virt", feature = "platform-ls2k1000"))]
compile_error!("Cannot compile for multiple platforms simultaneously");