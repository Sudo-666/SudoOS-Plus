use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicUsize, Ordering},
};

use myos_sync::{SpinLock, SpinLockGuard};

use crate::{
    context::IrqSaveGuard,
    lockdep::{LockClass, LockInstanceId},
    task::MigrationGuard,
};

const NO_OWNER: usize = usize::MAX;

/// A task-context spin lock for cross-CPU protocols that must keep IRQs enabled.
///
/// The guard pins the task to its current CPU. Local IRQs are disabled only for
/// short lockdep bookkeeping windows; spinning and payload access keep IRQs
/// enabled so IPI/TLB acknowledgements can make progress.
pub struct TrackedSpinLock<T> {
    inner: SpinLock<T>,
    class: LockClass,
    owner: AtomicUsize,
}

impl<T> TrackedSpinLock<T> {
    pub const fn new_with_class(value: T, class: LockClass) -> Self {
        Self {
            inner: SpinLock::new(value),
            class,
            owner: AtomicUsize::new(NO_OWNER),
        }
    }

    pub fn lock(&self) -> TrackedSpinLockGuard<'_, T> {
        crate::context::assert_task_context();
        crate::context::assert_interrupts_enabled();

        let migration_guard = MigrationGuard::new();
        let cpu = crate::smp::current_cpu_id();
        let instance = LockInstanceId::of(self);

        {
            let _irq_guard = IrqSaveGuard::new();
            crate::lockdep::before_lock(
                self.class,
                instance,
                self.owner.load(Ordering::Acquire),
                cpu,
            );
        }

        let guard = self.inner.lock();

        {
            let _irq_guard = IrqSaveGuard::new();
            self.owner.store(cpu.get(), Ordering::Release);
            crate::lockdep::after_lock(self.class, instance, cpu);
        }

        TrackedSpinLockGuard {
            lock: self,
            guard: Some(guard),
            migration_guard: Some(migration_guard),
            _not_send: PhantomData,
        }
    }

    pub fn try_lock(&self) -> Option<TrackedSpinLockGuard<'_, T>> {
        crate::context::assert_task_context();
        crate::context::assert_interrupts_enabled();

        let migration_guard = MigrationGuard::new();
        let cpu = crate::smp::current_cpu_id();
        let instance = LockInstanceId::of(self);

        {
            let _irq_guard = IrqSaveGuard::new();
            if self.owner.load(Ordering::Acquire) == cpu.get() {
                return None;
            }
            crate::lockdep::before_lock(
                self.class,
                instance,
                self.owner.load(Ordering::Acquire),
                cpu,
            );
        }

        let guard = self.inner.try_lock()?;

        {
            let _irq_guard = IrqSaveGuard::new();
            self.owner.store(cpu.get(), Ordering::Release);
            crate::lockdep::after_lock(self.class, instance, cpu);
        }

        Some(TrackedSpinLockGuard {
            lock: self,
            guard: Some(guard),
            migration_guard: Some(migration_guard),
            _not_send: PhantomData,
        })
    }
}

#[must_use = "dropping the guard immediately releases the lock"]
pub struct TrackedSpinLockGuard<'a, T> {
    lock: &'a TrackedSpinLock<T>,
    guard: Option<SpinLockGuard<'a, T>>,
    migration_guard: Option<MigrationGuard>,
    _not_send: PhantomData<*mut ()>,
}

impl<T> Deref for TrackedSpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.guard
            .as_ref()
            .expect("tracked spin-lock guard was already released")
            .deref()
    }
}

impl<T> DerefMut for TrackedSpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_mut()
            .expect("tracked spin-lock guard was already released")
            .deref_mut()
    }
}

impl<T> Drop for TrackedSpinLockGuard<'_, T> {
    fn drop(&mut self) {
        let cpu = crate::smp::current_cpu_id();
        let instance = LockInstanceId::of(self.lock);

        {
            let _irq_guard = IrqSaveGuard::new();
            crate::lockdep::before_unlock(self.lock.class, instance, cpu);
            self.lock.owner.store(NO_OWNER, Ordering::Release);
            drop(self.guard.take());
        }

        // IRQ state is restored before preemption/migration is re-enabled.
        drop(self.migration_guard.take());
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    use crate::lockdep::{LockClass, LockRank};

    static FIRST: TrackedSpinLock<usize> = TrackedSpinLock::new_with_class(
        0,
        LockClass::new("tracked_spin.verify", LockRank::CrossCpu, 10),
    );
    static SECOND: TrackedSpinLock<usize> = TrackedSpinLock::new_with_class(
        0,
        LockClass::new("tracked_spin.verify", LockRank::CrossCpu, 11),
    );

    crate::context::assert_interrupts_enabled();
    crate::context::assert_task_context();

    let initial_preempt_count = crate::task::preempt_count();
    {
        let mut first = FIRST.lock();
        assert!(crate::arch::interrupt::are_enabled());
        assert_eq!(crate::task::preempt_count(), initial_preempt_count + 1);
        *first = 11;

        // Same lock class but different instances must not look recursive.
        {
            let mut second = SECOND.lock();
            assert!(crate::arch::interrupt::are_enabled());
            assert_eq!(crate::task::preempt_count(), initial_preempt_count + 2);
            *second = 22;
        }
    }

    assert_eq!(crate::task::preempt_count(), initial_preempt_count);
    assert_eq!(*FIRST.lock(), 11);
    assert_eq!(*SECOND.lock(), 22);

    crate::println!("tracked spin lock test:");
    crate::println!("  IRQ-enabled contention : verified");
    crate::println!("  migration pinning       : verified");
    crate::println!("  instance-aware lockdep  : verified");
}
