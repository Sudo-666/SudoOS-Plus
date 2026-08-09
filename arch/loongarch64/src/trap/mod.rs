pub mod frame;

pub use frame::TrapFrame;

pub fn initialize() {
    // SAFETY: trap entry symbol 由本 crate 的汇编入口提供。
    unsafe { install_entry() }
}

unsafe fn install_entry() {
    // SAFETY: 写入当前 CPU 的 EENTRY/SAVE0 CSR，不访问 Rust 管理内存。
    unsafe {
        // 板级 (platform-ls2k1000)：EENTRY 指向专用页对齐 trampoline
        // (__loongarch_trap_entry_ls2k)，因为 LoongArch 硬件把 EENTRY[11:0]
        // 清零，普通 .text 里的共享入口无法保证 4 KiB 对齐。
        // 其余平台保持原样，编译产物逐字节不变。
        #[cfg(feature = "platform-ls2k1000")]
        core::arch::asm!(
            "la.pcrel $r12, __loongarch_trap_entry_ls2k",
            "csrwr $r12, 0xc",
            "csrwr $r0, 0x30",
            options(nostack),
        );

        #[cfg(not(feature = "platform-ls2k1000"))]
        core::arch::asm!(
            "la.pcrel $r12, __loongarch_trap_entry",
            "csrwr $r12, 0xc",
            "csrwr $r0, 0x30",
            options(nostack),
        );
    }
}

/// 板级异常向量符号地址（应由链接脚本保证页对齐）。
#[cfg(feature = "platform-ls2k1000")]
pub fn ls2k_eentry_expected() -> usize {
    // SAFETY: 纯符号地址读取，不访问内存。
    unsafe { __loongarch_trap_entry_ls2k as usize }
}

/// 回读当前 CPU 的 EENTRY CSR（硬件已把低 12 位清零）。
#[cfg(feature = "platform-ls2k1000")]
pub fn ls2k_eentry_installed() -> usize {
    let value: usize;
    // SAFETY: 只读取当前 CPU 的 EENTRY CSR。
    unsafe {
        core::arch::asm!(
            "csrrd {value}, 0xc",
            value = out(reg) value,
            options(nostack),
        );
    }
    value
}

/// 回读当前 CPU 的 ECFG CSR（VS 字段 = bits[18:16]，0 表示无向量表，
/// 所有普通异常/中断都进入 EENTRY）。
#[cfg(feature = "platform-ls2k1000")]
pub fn ls2k_ecfg() -> usize {
    let value: usize;
    // SAFETY: 只读取当前 CPU 的 ECFG CSR。
    unsafe {
        core::arch::asm!(
            "csrrd {value}, 0x4",
            value = out(reg) value,
            options(nostack),
        );
    }
    value
}

pub fn trigger_breakpoint() {
    // SAFETY: 故意触发同步 breakpoint，用于验证 trap entry。
    unsafe {
        core::arch::asm!("break 0", options(nostack));
    }
}

pub fn kernel_scratch_is_clean() -> bool {
    let scratch: usize;
    // SAFETY: 只读取当前 CPU 的 SAVE0 CSR。
    unsafe {
        core::arch::asm!(
            "csrrd {scratch}, 0x30",
            scratch = out(reg) scratch,
            options(nomem, nostack),
        );
    }
    scratch == 0
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

#[cfg(feature = "platform-ls2k1000")]
unsafe extern "C" {
    fn __loongarch_trap_entry_ls2k();
}

#[unsafe(no_mangle)]
extern "C" fn kernel_loongarch_trap(frame: &mut TrapFrame) {
    // SAFETY: kernel_arch_trap 由 kernel crate 提供，是架构 trap 入口的公共调度点。
    unsafe { kernel_arch_trap(frame) }
}
