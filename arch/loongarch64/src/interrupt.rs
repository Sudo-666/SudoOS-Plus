use core::arch::asm;

const CRMD_IE: usize = 1 << 2;
const CSR_ECFG: usize = 0x4;
const ECFG_INTERRUPT_MASK: usize = 0x1fff;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "saved interrupt state should be restored"]
pub struct InterruptState {
    enabled: bool,
}

impl InterruptState {
    pub const fn was_enabled(self) -> bool {
        self.enabled
    }
}

fn exchange_interrupt_enable(new_value: usize) -> usize {
    let mut value = new_value;

    // SAFETY: csrxchg 只修改当前 CPU 的 CRMD.IE 位并返回旧 CRMD。
    unsafe {
        asm!(
            "csrxchg {value}, $r12, 0x0",
            value = inout(reg) value,
            in("$r12") CRMD_IE,
            options(nostack),
        );
    }

    value
}

pub fn save_and_disable() -> InterruptState {
    let previous = exchange_interrupt_enable(0);

    InterruptState {
        enabled: previous & CRMD_IE != 0,
    }
}

pub fn restore(state: InterruptState) {
    let value = if state.was_enabled() { CRMD_IE } else { 0 };

    exchange_interrupt_enable(value);
}

pub fn disable() {
    exchange_interrupt_enable(0);
}

/// # Safety
///
/// 调用者必须保证异常入口、栈以及中断控制器已经正确配置。
pub unsafe fn enable() {
    exchange_interrupt_enable(CRMD_IE);
}

pub fn mask_all_sources() {
    let value: usize = 0;

    // SAFETY: clear only ECFG.LIE[12:0], preserving vector-spacing and all
    // reserved fields.  r12 is used only as the CSRXCHG mask operand.
    unsafe {
        asm!(
            "csrxchg {value}, $r12, {csr}",
            value = inout(reg) value => _,
            in("$r12") ECFG_INTERRUPT_MASK,
            csr = const CSR_ECFG,
            options(nomem, nostack),
        );
    }
}

pub fn are_enabled() -> bool {
    let value: usize;

    // SAFETY: 只读取当前 CPU 的 CRMD CSR。
    unsafe {
        asm!(
            "csrrd {value}, 0x0",
            value = out(reg) value,
            options(nostack),
        );
    }

    value & CRMD_IE != 0
}

pub fn are_disabled() -> bool {
    !are_enabled()
}
