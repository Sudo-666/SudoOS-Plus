pub mod frame;

pub use frame::TrapFrame;

pub fn initialize() {
    // SAFETY: trap entry symbol 由本 crate 的汇编入口提供。
    unsafe { install_entry() }
}

unsafe fn install_entry() {
    // SAFETY: 写入当前 hart 的 stvec/sscratch，不访问 Rust 管理内存。
    unsafe {
        core::arch::asm!(
            "la t0, __riscv_trap_entry",
            "csrw stvec, t0",
            "csrw sscratch, zero",
            options(nostack),
        );
    }
}

pub fn trigger_breakpoint() {
    // SAFETY: 故意触发 32 位同步 breakpoint，用于验证 trap entry。
    unsafe {
        core::arch::asm!(
            ".option push",
            ".option norvc",
            ".4byte 0x00100073",
            ".option pop",
            options(nostack),
        );
    }
}

pub fn kernel_scratch_is_clean() -> bool {
    let scratch: usize;

    // SAFETY: 只读取当前 hart 的 sscratch CSR。
    unsafe {
        core::arch::asm!(
            "csrr {scratch}, sscratch",
            scratch = out(reg) scratch,
            options(nomem, nostack),
        );
    }

    scratch == 0
}

#[cfg(debug_assertions)]
pub fn verify_register_restore() -> bool {
    // SAFETY: 汇编函数遵循 C ABI，并完整恢复所有被调用者保存寄存器。
    unsafe { __riscv_trap_register_self_test() != 0 }
}

unsafe extern "C" {
    fn kernel_arch_trap(frame: &mut TrapFrame);

    #[cfg(debug_assertions)]
    fn __riscv_trap_register_self_test() -> usize;
}

#[unsafe(no_mangle)]
extern "C" fn kernel_riscv_trap(frame: &mut TrapFrame) {
    // SAFETY: kernel_arch_trap 由 kernel crate 提供，是架构 trap 入口的公共调度点。
    unsafe { kernel_arch_trap(frame) }
}
