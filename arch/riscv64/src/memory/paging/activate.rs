use core::arch::asm;

use myos_mm::{PAGE_SHIFT, PhysFrame};

const SATP_MODE_SHIFT: usize = 60;
const SATP_MODE_BARE: usize = 0;
const SATP_MODE_SV39: usize = 8;

const SATP_MODE_MASK: usize = 0xf << SATP_MODE_SHIFT;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivateError {
    RootAddressOutOfRange,
    ModeRejected { actual_mode: usize },
}

pub fn current_satp() -> usize {
    let value: usize;

    // SAFETY:
    // 只读取 supervisor 地址翻译控制寄存器。
    unsafe {
        asm!(
            "csrr {value}, satp",
            value = out(reg) value,
            options(nomem, nostack),
        );
    }

    value
}

pub fn current_mode() -> usize {
    (current_satp() & SATP_MODE_MASK) >> SATP_MODE_SHIFT
}

pub fn translation_is_enabled() -> bool {
    current_mode() != SATP_MODE_BARE
}

/// 将当前临时 Sv39 根页表替换为正式内核根页表。
///
/// 当前执行流已经位于高半内核。
///
/// # Safety
///
/// 调用者必须保证新的根页表包含：
///
/// - 当前高地址代码；
/// - 当前高地址栈和数据；
/// - early UART；
/// - 页表自身所需的 RAM direct map。
#[inline(never)]
pub unsafe fn switch_sv39_root(root: PhysFrame) -> Result<(), ActivateError> {
    if current_mode() != SATP_MODE_SV39 {
        return Err(ActivateError::ModeRejected {
            actual_mode: current_mode(),
        });
    }

    let root_address = root.start_address().get();

    let root_ppn = root_address >> PAGE_SHIFT;

    if root_ppn >= (1_usize << 44) {
        return Err(ActivateError::RootAddressOutOfRange);
    }

    let satp = (SATP_MODE_SV39 << SATP_MODE_SHIFT) | root_ppn;

    // SAFETY: 新页表映射当前执行地址和 trap landing，root 已经通过范围校验。
    unsafe {
        asm!(
            /*
             * 当前函数本身位于高半地址，并且新旧页表都映射
             * 完全相同的高半内核地址。
             */
            "la t0, 2f",
            "csrw stvec, t0",

            /*
             * 确保正式页表的所有写入已对 walker 可见。
             */
            "sfence.vma zero, zero",

            /*
             * 替换临时静态根页表。
             */
            "csrw satp, {satp}",

            /*
             * 清除旧临时页表产生的翻译缓存。
             */
            "sfence.vma zero, zero",
            "fence.i",

            "j 3f",

            "2:",
            "wfi",
            "j 2b",

            "3:",

            satp = in(reg) satp,
            lateout("t0") _,
            options(nostack),
        );
    }

    Ok(())
}
