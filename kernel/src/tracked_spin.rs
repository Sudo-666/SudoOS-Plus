use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::AtomicUsize,
};

#[cfg(debug_assertions)]
use core::sync::atomic::Ordering;

use myos_sync::{SpinLock, SpinLockGuard};

use crate::{
    lockdep::LockClass,
    task::{MigrationGuard, PreemptGuard},
};

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

// Preemptible tracked locks (pin_migration == false) take no migration guard,
// so their holders keep preempt_count == 0 and a scheduler could switch them
// away mid-critical-section. The per-CPU LIFO lockdep stack requires that a
// holder is never switched away: a later task would push its own locks on top
// of the suspended holder's entries, and the holder's eventual release would
// pop the wrong instance. This depth lets the scheduler defer switches while
// such a lock is held (debug builds only; release has no lockdep stack to
// corrupt, and holding a spin lock across a switch is safe for the lock).
#[cfg(debug_assertions)]
static PREEMPTIBLE_DEPTH: [AtomicUsize; crate::smp::MAX_CPUS] =
    [const { AtomicUsize::new(0) }; crate::smp::MAX_CPUS];

#[cfg(debug_assertions)]
fn record_preemptible_acquire() {
    let cpu = crate::smp::current_cpu_id();
    PREEMPTIBLE_DEPTH[cpu.get()].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

#[cfg(debug_assertions)]
fn record_preemptible_release() {
    let cpu = crate::smp::current_cpu_id();
    let depth = PREEMPTIBLE_DEPTH[cpu.get()].fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
    assert!(depth >= 1, "preemptible tracked-lock depth underflowed");
}

#[cfg(debug_assertions)]
pub fn preemptible_lock_depth(cpu: crate::smp::CpuId) -> usize {
    PREEMPTIBLE_DEPTH[cpu.get()].load(core::sync::atomic::Ordering::Relaxed)
}

/// Enables local interrupts for the duration of a preemptible-lock spin and
/// restores the caller's state right after acquisition. The critical section
/// itself runs in the caller's original IRQ state; only the wait is
/// interruptible, so the CPU can service the timer, IPIs, and TLB ACKs while
/// the holder (possibly preempted off this CPU) makes progress elsewhere.
struct SpinIrqWindow(crate::arch::interrupt::InterruptState);

impl Drop for SpinIrqWindow {
    fn drop(&mut self) {
        crate::arch::interrupt::restore(self.0);
        crate::task::note_irq_state(self.0.was_enabled());
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

        // pin_migration locks pair the migration pin with a preemption pin:
        // the per-CPU LIFO lockdep stack requires the holder to never be
        // switched mid-critical-section, and the migration pin alone (which
        // stays preemptible) would not guarantee that. The guard drops the
        // payload lock first, then migration, then preemption.
        let migration_guard = (self.pin_migration
            && crate::task::scheduler_is_initialized()
            && crate::arch::interrupt::are_enabled())
        .then(MigrationGuard::new);
        let preempt_guard = migration_guard.is_some().then(PreemptGuard::new);
        #[cfg(debug_assertions)]
        let (mut cpu, instance) = {
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

        // §10 情况A: a preemptible-lock waiter must spin with local interrupts
        // enabled. A trap-migrated task can resume with SIE=0 carried across
        // the context switch; spinning IRQ-off on a lock held by a preempted
        // holder leaves the CPU deaf to the timer, IPIs, and TLB ACKs until
        // the holder runs again — a 30 s deaf window that TLB shootdown
        // deadlines cannot survive (BuildStorm runs 4/5, cpu0 pending=0x3).
        // Enabling IRQs for the spin only makes the wait interruptible; the
        // critical section itself runs in the caller's original IRQ state.
        let spin_irq_window = (!self.pin_migration && crate::arch::interrupt::are_disabled())
            .then(|| {
                // SAFETY: syscall/trap context on the current task's kernel
                // stack; the saved state is restored after acquisition.
                let state = crate::arch::interrupt::save_and_disable();
                unsafe { crate::arch::interrupt::enable() };
                crate::task::note_irq_state(true);
                SpinIrqWindow(state)
            });

        let guard = self.inner.lock();
        drop(spin_irq_window);
        if migration_guard.is_some() {
            record_acquire(self.class);
        }
        crate::task::note_tracked_lock_acquire();
        #[cfg(debug_assertions)]
        {
            let _irq_guard = IrqSaveGuard::new();
            if !self.pin_migration {
                // A preemptible-lock waiter keeps IRQs enabled and may be
                // preempted (and migrated) while spinning. Re-resolve the CPU
                // with IRQs off so the depth increment, the lockdep push, and
                // the owner stamp all land on the stack of the CPU that runs
                // the critical section, atomically with respect to the preempt
                // gate. A timer landing between the re-resolve and the depth
                // increment would otherwise pass the gate (depth still 0) and
                // switch the new holder mid-acquire; it would resume on another
                // CPU, pushing user_mm on the stale CPU's lockdep stack and
                // leaving a dangling entry there.
                cpu = crate::smp::current_cpu_id();
                record_preemptible_acquire();
            }
            self.owner.store(cpu.get(), Ordering::Release);
            crate::lockdep::after_lock(self.class, instance, cpu);
        }

        TrackedSpinLockGuard {
            lock: self,
            guard: Some(guard),
            migration_guard,
            preempt_guard,
            _not_send: PhantomData,
        }
    }

    /// P0-2A: exclusive access for an exclusively borrowed lock object.
    ///
    /// `&mut self` proves that no safe concurrent locker/guard can exist,
    /// therefore taking the payload without acquiring the runtime spin lock
    /// is sound. Intended for object final destruction / unpublished setup
    /// so that a final Arc drop never becomes a hidden cross-CPU lock
    /// acquisition.
    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: the exclusive borrow of TrackedSpinLock excludes every
        // safe concurrent lock()/try_lock() caller and guard.
        unsafe { self.inner.get_mut_unchecked() }
    }

    pub fn try_lock(&self) -> Option<TrackedSpinLockGuard<'_, T>> {
        #[cfg(debug_assertions)]
        {
            if crate::task::scheduler_is_initialized() && crate::arch::interrupt::are_enabled() {
                crate::context::assert_task_context();
                crate::lockdep::assert_irq_enabled_outer_lock(self.class);
            }
        }

        // pin_migration locks pair the migration pin with a preemption pin:
        // the per-CPU LIFO lockdep stack requires the holder to never be
        // switched mid-critical-section, and the migration pin alone (which
        // stays preemptible) would not guarantee that. The guard drops the
        // payload lock first, then migration, then preemption.
        let migration_guard = (self.pin_migration
            && crate::task::scheduler_is_initialized()
            && crate::arch::interrupt::are_enabled())
        .then(MigrationGuard::new);
        let preempt_guard = migration_guard.is_some().then(PreemptGuard::new);
        #[cfg(debug_assertions)]
        let (mut cpu, instance) = {
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
        crate::task::note_tracked_lock_acquire();
        #[cfg(debug_assertions)]
        {
            let _irq_guard = IrqSaveGuard::new();
            if !self.pin_migration {
                // See lock(): re-resolve with IRQs off so the depth increment
                // and the lockdep push land on the critical-section CPU,
                // atomically with respect to the preempt gate.
                cpu = crate::smp::current_cpu_id();
                record_preemptible_acquire();
            }
            self.owner.store(cpu.get(), Ordering::Release);
            crate::lockdep::after_lock(self.class, instance, cpu);
        }

        Some(TrackedSpinLockGuard {
            lock: self,
            guard: Some(guard),
            migration_guard,
            preempt_guard,
            _not_send: PhantomData,
        })
    }
}

#[must_use = "dropping the guard immediately releases the lock"]
pub struct TrackedSpinLockGuard<'a, T> {
    lock: &'a TrackedSpinLock<T>,
    guard: Option<SpinLockGuard<'a, T>>,
    migration_guard: Option<MigrationGuard>,
    preempt_guard: Option<PreemptGuard>,
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
            let _irq_guard = IrqSaveGuard::new();
            // The preempt gate defers switches while the preemptible depth is
            // non-zero, and the decrement below runs inside this IRQ-off
            // window, so the release CPU is necessarily the CPU that pushed
            // this lock onto the lockdep stack.
            let cpu = crate::smp::current_cpu_id();
            let instance = LockInstanceId::of(self.lock);
            crate::lockdep::before_unlock(self.lock.class, instance, cpu);
            self.lock.owner.store(NO_OWNER, Ordering::Release);
            if !self.lock.pin_migration {
                record_preemptible_release();
            }
        }

        drop(self.guard.take());

        crate::task::note_tracked_lock_release();

        if self.migration_guard.is_some() {
            record_release();
        }

        // The payload lock is released before migration is re-enabled, and
        // migration before preemption, so the holder stays fixed to this CPU
        // for the whole critical section and its exit bookkeeping.
        drop(self.migration_guard.take());
        drop(self.preempt_guard.take());
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
