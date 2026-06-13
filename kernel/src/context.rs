use core::marker::PhantomData;

#[must_use = "dropping the guard restores the saved interrupt state"]
pub struct IrqSaveGuard {
    state: crate::arch::interrupt::InterruptState,
    disabled_at: u64,
    _not_send: PhantomData<*mut ()>,
}

impl IrqSaveGuard {
    pub fn new() -> Self {
        Self {
            state: crate::arch::interrupt::save_and_disable(),
            disabled_at: crate::arch::time::counter(),
            _not_send: PhantomData,
        }
    }
}

impl Default for IrqSaveGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for IrqSaveGuard {
    fn drop(&mut self) {
        if self.state.was_enabled() {
            crate::lockdep::record_irq_off(
                crate::arch::time::counter().wrapping_sub(self.disabled_at),
            );
        }
        crate::arch::interrupt::restore(self.state);
    }
}

pub fn assert_interrupts_enabled() {
    assert!(
        crate::arch::interrupt::are_enabled(),
        "operation requires local interrupts enabled",
    );
}

pub fn assert_interrupts_disabled() {
    assert!(
        crate::arch::interrupt::are_disabled(),
        "operation requires local interrupts disabled",
    );
}

pub fn in_irq() -> bool {
    crate::task::irq_depth() != 0
}

pub fn irq_depth() -> usize {
    crate::task::irq_depth()
}

pub fn preempt_count() -> usize {
    crate::task::preempt_count()
}

pub fn assert_task_context() {
    assert!(!in_irq(), "operation is not allowed in IRQ context");
}

#[allow(dead_code)]
pub fn assert_irq_context() {
    assert_ne!(irq_depth(), 0, "operation requires IRQ context");
}

pub fn might_sleep() {
    assert_task_context();
    assert_eq!(
        preempt_count(),
        0,
        "operation may sleep with preemption disabled",
    );
    assert_interrupts_enabled();
}
