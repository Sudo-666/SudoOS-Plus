// 平台选择:严格互斥。
//
// arch-riscv64 不再带默认平台 feature。kernel 以 default-features=false
// 依赖本 crate,平台由构建脚本按 ARCH+PLATFORM 显式启用恰好一个 feature;
// 同时启用或全部未启用都会在编译期报错(compile_error!),杜绝"VF2 优先"
// 之类掩盖多平台混编的写法。

#[cfg(feature = "platform-qemu-virt")]
mod qemu_virt;

#[cfg(feature = "platform-qemu-virt")]
pub(crate) use qemu_virt::{boot_context, reserve_early_memory, write_console_byte};

#[cfg(feature = "platform-visionfive2")]
mod visionfive2;

#[cfg(feature = "platform-visionfive2")]
pub(crate) use visionfive2::{boot_context, reserve_early_memory, write_console_byte};

// 防止漏选平台
#[cfg(not(any(feature = "platform-qemu-virt", feature = "platform-visionfive2")))]
compile_error!("Please select a RISC-V platform feature (e.g. platform-qemu-virt)");

// 防止多选平台导致冲突
#[cfg(all(feature = "platform-qemu-virt", feature = "platform-visionfive2"))]
compile_error!("Cannot compile for multiple RISC-V platforms simultaneously");
