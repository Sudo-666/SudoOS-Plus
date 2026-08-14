use core::marker::PhantomData;

#[must_use = "dropping the guard restores the saved interrupt state"]
pub struct IrqSaveGuard {
    state: crate::arch::interrupt::InterruptState,
    #[cfg(debug_assertions)]
    disabled_at: u64,
    _not_send: PhantomData<*mut ()>,
}

impl IrqSaveGuard {
    pub fn new() -> Self {
        Self {
            state: crate::arch::interrupt::save_and_disable(),
            #[cfg(debug_assertions)]
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
        #[cfg(debug_assertions)]
        {
            if self.state.was_enabled() {
                crate::lockdep::record_irq_off(
                    crate::arch::time::counter().wrapping_sub(self.disabled_at),
                );
            }
        }
        crate::arch::interrupt::restore(self.state);
    }
}

#[track_caller]
pub fn assert_interrupts_enabled() {
    assert!(
        crate::arch::interrupt::are_enabled(),
        "operation requires local interrupts enabled",
    );
}

#[track_caller]
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

#[track_caller]
pub fn assert_task_context() {
    assert!(!in_irq(), "operation is not allowed in IRQ context");
}

#[allow(dead_code)]
pub fn assert_irq_context() {
    assert_ne!(irq_depth(), 0, "operation requires IRQ context");
}

#[track_caller]
pub fn might_sleep() {
    assert_task_context();
    let cpu = crate::smp::current_cpu_id();
    let tracked = crate::tracked_spin::held_diagnostic(cpu);
    let task = crate::task::current_task_diagnostic();
    assert_eq!(
        preempt_count(),
        0,
        "operation may sleep with preemption disabled: cpu={} task={:?} kind={} task_preempt={} tracked_depth={} tracked_key={}",
        cpu.get(),
        task.0,
        task.1,
        task.2,
        tracked.0,
        tracked.1,
    );
    assert_interrupts_enabled();
}
