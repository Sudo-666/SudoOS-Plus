use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::AtomicUsize,
};

#[cfg(debug_assertions)]
use core::sync::atomic::Ordering;

use myos_sync::{SpinLock, SpinLockGuard};

use crate::{lockdep::LockClass, task::MigrationGuard};

#[cfg(debug_assertions)]
use crate::{context::IrqSaveGuard, lockdep::LockInstanceId};

const NO_OWNER: usize = usize::MAX;
const DIAGNOSTIC_DEPTH: usize = 4;
static HELD_DEPTH: [AtomicUsize; crate::smp::MAX_CPUS] =
    [const { AtomicUsize::new(0) }; crate::smp::MAX_CPUS];
static HELD_KEYS: [[AtomicUsize; DIAGNOSTIC_DEPTH]; crate::smp::MAX_CPUS] =
    [const { [const { AtomicUsize::new(0) }; DIAGNOSTIC_DEPTH] }; crate::smp::MAX_CPUS];

pub fn held_diagnostic(cpu: crate::smp::CpuId) -> (usize, usize) {
    let depth = HELD_DEPTH[cpu.get()].load(core::sync::atomic::Ordering::Relaxed);
    let key = depth
        .checked_sub(1)
        .filter(|index| *index < DIAGNOSTIC_DEPTH)
        .map_or(0, |index| {
            HELD_KEYS[cpu.get()][index].load(core::sync::atomic::Ordering::Relaxed)
        });
    (depth, key)
}

fn record_acquire(class: LockClass) {
    let cpu = crate::smp::current_cpu_id();
    let depth = HELD_DEPTH[cpu.get()].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    if depth < DIAGNOSTIC_DEPTH {
        let key = (class.rank as usize) * 1024 + class.order;
        HELD_KEYS[cpu.get()][depth].store(key, core::sync::atomic::Ordering::Relaxed);
    }
}

fn record_release() {
    let cpu = crate::smp::current_cpu_id();
    let depth = HELD_DEPTH[cpu.get()].fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
    if depth != 0 && depth <= DIAGNOSTIC_DEPTH {
        HELD_KEYS[cpu.get()][depth - 1].store(0, core::sync::atomic::Ordering::Relaxed);
    }
}

/// A preemption-safe spin lock for cross-CPU protocols that may keep IRQs enabled.
///
/// In normal task context the guard pins the task to its current CPU while
/// spinning and accessing the payload, allowing IPI/TLB acknowledgements to
/// make progress. Early boot and already-IRQ-disabled trap/switch paths cannot
/// migrate and therefore do not need to enter the scheduler for pinning.
pub struct TrackedSpinLock<T> {
    inner: SpinLock<T>,
    class: LockClass,
    pin_migration: bool,
    owner: AtomicUsize,
}

impl<T> TrackedSpinLock<T> {
    pub const fn new_with_class(value: T, class: LockClass) -> Self {
        Self {
            inner: SpinLock::new(value),
            class,
            pin_migration: true,
            owner: AtomicUsize::new(NO_OWNER),
        }
    }

    /// A preemptible data lock for state whose callers have an independent
    /// lifetime/CPU-ownership protocol. Waiters keep IRQs enabled and may be
    /// preempted while spinning, so the owner can run and release the lock.
    pub const fn new_preemptible(value: T, class: LockClass) -> Self {
        Self {
            inner: SpinLock::new(value),
            class,
            pin_migration: false,
            owner: AtomicUsize::new(NO_OWNER),
        }
    }

    pub fn lock(&self) -> TrackedSpinLockGuard<'_, T> {
        #[cfg(debug_assertions)]
        {
            if crate::task::scheduler_is_initialized() && crate::arch::interrupt::are_enabled() {
                crate::context::assert_task_context();
                crate::lockdep::assert_irq_enabled_outer_lock(self.class);
            }
        }

        let migration_guard = (self.pin_migration
            && crate::task::scheduler_is_initialized()
            && crate::arch::interrupt::are_enabled())
        .then(MigrationGuard::new);
        #[cfg(debug_assertions)]
        let (cpu, instance) = {
            let cpu = crate::smp::current_cpu_id();
            let instance = LockInstanceId::of(self);
            let _irq_guard = IrqSaveGuard::new();
            crate::lockdep::before_lock(
                self.class,
                instance,
                self.owner.load(Ordering::Acquire),
                cpu,
            );
            (cpu, instance)
        };

        let guard = self.inner.lock();
        if migration_guard.is_some() {
            record_acquire(self.class);
        }

        #[cfg(debug_assertions)]
        {
            let _irq_guard = IrqSaveGuard::new();
            self.owner.store(cpu.get(), Ordering::Release);
            crate::lockdep::after_lock(self.class, instance, cpu);
        }

        TrackedSpinLockGuard {
            lock: self,
            guard: Some(guard),
            migration_guard,
            _not_send: PhantomData,
        }
    }

    pub fn try_lock(&self) -> Option<TrackedSpinLockGuard<'_, T>> {
        #[cfg(debug_assertions)]
        {
            if crate::task::scheduler_is_initialized() && crate::arch::interrupt::are_enabled() {
                crate::context::assert_task_context();
                crate::lockdep::assert_irq_enabled_outer_lock(self.class);
            }
        }

        let migration_guard = (self.pin_migration
            && crate::task::scheduler_is_initialized()
            && crate::arch::interrupt::are_enabled())
        .then(MigrationGuard::new);
        #[cfg(debug_assertions)]
        let (cpu, instance) = {
            let cpu = crate::smp::current_cpu_id();
            let instance = LockInstanceId::of(self);
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
            (cpu, instance)
        };

        let guard = self.inner.try_lock()?;
        if migration_guard.is_some() {
            record_acquire(self.class);
        }

        #[cfg(debug_assertions)]
        {
            let _irq_guard = IrqSaveGuard::new();
            self.owner.store(cpu.get(), Ordering::Release);
            crate::lockdep::after_lock(self.class, instance, cpu);
        }

        Some(TrackedSpinLockGuard {
            lock: self,
            guard: Some(guard),
            migration_guard,
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
        #[cfg(debug_assertions)]
        {
            let cpu = crate::smp::current_cpu_id();
            let instance = LockInstanceId::of(self.lock);
            let _irq_guard = IrqSaveGuard::new();
            crate::lockdep::before_unlock(self.lock.class, instance, cpu);
            self.lock.owner.store(NO_OWNER, Ordering::Release);
        }

        drop(self.guard.take());

        if self.migration_guard.is_some() {
            record_release();
        }

        // The payload lock is released before preemption/migration is re-enabled.
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
