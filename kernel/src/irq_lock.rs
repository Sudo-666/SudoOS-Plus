use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicUsize, Ordering},
};

use myos_sync::{SpinLock, SpinLockGuard};

use crate::lockdep::{LockClass, LockInstanceId, LockRank};

const NO_OWNER: usize = usize::MAX;

pub struct IrqSpinLock<T> {
    inner: SpinLock<T>,
    class: LockClass,
    owner: AtomicUsize,
}

impl<T> IrqSpinLock<T> {
    pub const fn new_with_class(value: T, class: LockClass) -> Self {
        Self {
            inner: SpinLock::new(value),
            class,
            owner: AtomicUsize::new(NO_OWNER),
        }
    }
}

impl<T> IrqSpinLock<T> {
    pub fn lock(&self) -> IrqSpinLockGuard<'_, T> {
        let interrupt_guard = crate::context::IrqSaveGuard::new();
        let cpu = crate::smp::current_cpu_id();
        crate::lockdep::before_lock(
            self.class,
            LockInstanceId::of(self),
            self.owner.load(Ordering::Acquire),
            cpu,
        );

        let guard = self.inner.lock();
        self.owner.store(cpu.get(), Ordering::Release);
        crate::lockdep::after_lock(self.class, LockInstanceId::of(self), cpu);

        IrqSpinLockGuard {
            lock: self,
            guard: Some(guard),
            _interrupt_guard: interrupt_guard,
            _not_send: PhantomData,
        }
    }

    pub fn try_lock(&self) -> Option<IrqSpinLockGuard<'_, T>> {
        let interrupt_guard = crate::context::IrqSaveGuard::new();
        let cpu = crate::smp::current_cpu_id();
        if self.owner.load(Ordering::Acquire) == cpu.get() {
            return None;
        }
        crate::lockdep::before_lock(
            self.class,
            LockInstanceId::of(self),
            self.owner.load(Ordering::Acquire),
            cpu,
        );

        match self.inner.try_lock() {
            Some(guard) => {
                self.owner.store(cpu.get(), Ordering::Release);
                crate::lockdep::after_lock(self.class, LockInstanceId::of(self), cpu);
                Some(IrqSpinLockGuard {
                    lock: self,
                    guard: Some(guard),
                    _interrupt_guard: interrupt_guard,
                    _not_send: PhantomData,
                })
            }

            None => None,
        }
    }
}

#[must_use = "dropping the guard immediately releases the lock"]
pub struct IrqSpinLockGuard<'a, T> {
    lock: &'a IrqSpinLock<T>,
    guard: Option<SpinLockGuard<'a, T>>,

    _interrupt_guard: crate::context::IrqSaveGuard,

    _not_send: PhantomData<*mut ()>,
}

impl<T> Deref for IrqSpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.guard
            .as_ref()
            .expect("IRQ spin-lock guard was already released")
            .deref()
    }
}

impl<T> DerefMut for IrqSpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_mut()
            .expect("IRQ spin-lock guard was already released")
            .deref_mut()
    }
}

impl<T> Drop for IrqSpinLockGuard<'_, T> {
    fn drop(&mut self) {
        let cpu = crate::smp::current_cpu_id();
        crate::lockdep::before_unlock(self.lock.class, LockInstanceId::of(self.lock), cpu);
        self.lock.owner.store(NO_OWNER, Ordering::Release);
        drop(self.guard.take());
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    static FIRST: IrqSpinLock<usize> = IrqSpinLock::new_with_class(
        0,
        LockClass::new("irq_lock.verify.first", LockRank::Scheduler, 1),
    );
    static SECOND: IrqSpinLock<usize> = IrqSpinLock::new_with_class(
        0,
        LockClass::new("irq_lock.verify.second", LockRank::WaitQueue, 1),
    );

    let initially_enabled = crate::arch::interrupt::are_enabled();

    {
        let mut first = FIRST.lock();

        assert!(crate::arch::interrupt::are_disabled());

        *first = 11;

        assert!(FIRST.try_lock().is_none());
        assert!(crate::arch::interrupt::are_disabled());

        {
            let mut second = SECOND.lock();
            assert!(crate::arch::interrupt::are_disabled());
            *second = 22;
        }

        assert!(crate::arch::interrupt::are_disabled());
    }

    assert_eq!(
        crate::arch::interrupt::are_enabled(),
        initially_enabled,
        "IRQ spin lock did not restore interrupt state",
    );

    assert_eq!(*FIRST.lock(), 11);
    assert_eq!(*SECOND.lock(), 22);

    crate::println!("IRQ spin lock test:");
    crate::println!("  local interrupt masking : verified");
    crate::println!("  nested restore          : verified");
    crate::println!("  failed try_lock restore : verified");
    crate::println!("  lockdep class/rank      : verified");
    crate::println!(
        "  max IRQ-off cycles      : {}",
        crate::lockdep::max_irq_off_cycles(),
    );
    crate::println!(
        "  scheduler hold cycles   : {}",
        crate::lockdep::max_hold_cycles(LockRank::Scheduler),
    );
    crate::println!(
        "  console hold cycles     : {}",
        crate::lockdep::max_hold_cycles(LockRank::Console),
    );
}
