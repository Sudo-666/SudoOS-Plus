pub mod frame;

pub use frame::TrapFrame;

pub fn initialize() {
    // SAFETY: trap entry symbol 由本 crate 的汇编入口提供。
    unsafe { install_entry() }
}

unsafe fn install_entry() {
    // SAFETY: 写入当前 CPU 的 EENTRY CSR，不访问 Rust 管理内存。
    unsafe {
        core::arch::asm!(
            "la.pcrel $r12, __loongarch_trap_entry",
            "csrwr $r12, 0xc",
            options(nostack),
        );
    }
}

pub fn trigger_breakpoint() {
    // SAFETY: 故意触发同步 breakpoint，用于验证 trap entry。
    unsafe {
        core::arch::asm!("break 0", options(nostack));
    }
}

pub const fn kernel_scratch_is_clean() -> bool {
    // LoongArch 当前只允许内核态异常；用户态换栈协议尚未启用。
    true
}

#[cfg(debug_assertions)]
pub fn verify_register_restore() -> bool {
    // SAFETY: 汇编函数遵循 C ABI，并完整恢复所有被调用者保存寄存器。
    unsafe { __loongarch_trap_register_self_test() != 0 }
}

unsafe extern "C" {
    fn kernel_arch_trap(frame: &mut TrapFrame);

    #[cfg(debug_assertions)]
    fn __loongarch_trap_register_self_test() -> usize;
}

#[unsafe(no_mangle)]
extern "C" fn kernel_loongarch_trap(frame: &mut TrapFrame) {
    // SAFETY: kernel_arch_trap 由 kernel crate 提供，是架构 trap 入口的公共调度点。
    unsafe { kernel_arch_trap(frame) }
}
