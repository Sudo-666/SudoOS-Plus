use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use myos_sync::{SpinLock, SpinLockGuard};

pub struct IrqSpinLock<T: ?Sized> {
    inner: SpinLock<T>,
}

impl<T> IrqSpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: SpinLock::new(value),
        }
    }
}

impl<T: ?Sized> IrqSpinLock<T> {
    pub fn lock(&self) -> IrqSpinLockGuard<'_, T> {
        let interrupt_state = crate::arch::interrupt::save_and_disable();

        let guard = self.inner.lock();

        IrqSpinLockGuard {
            guard: Some(guard),
            interrupt_state,
            _not_send: PhantomData,
        }
    }

    pub fn try_lock(&self) -> Option<IrqSpinLockGuard<'_, T>> {
        let interrupt_state = crate::arch::interrupt::save_and_disable();

        match self.inner.try_lock() {
            Some(guard) => Some(IrqSpinLockGuard {
                guard: Some(guard),
                interrupt_state,
                _not_send: PhantomData,
            }),

            None => {
                crate::arch::interrupt::restore(interrupt_state);
                None
            }
        }
    }
}

#[must_use = "dropping the guard immediately releases the lock"]
pub struct IrqSpinLockGuard<'a, T: ?Sized> {
    guard: Option<SpinLockGuard<'a, T>>,

    interrupt_state: crate::arch::interrupt::InterruptState,

    _not_send: PhantomData<*mut ()>,
}

impl<T: ?Sized> Deref for IrqSpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.guard
            .as_ref()
            .expect("IRQ spin-lock guard was already released")
            .deref()
    }
}

impl<T: ?Sized> DerefMut for IrqSpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_mut()
            .expect("IRQ spin-lock guard was already released")
            .deref_mut()
    }
}

impl<T: ?Sized> Drop for IrqSpinLockGuard<'_, T> {
    fn drop(&mut self) {
        drop(self.guard.take());
        crate::arch::interrupt::restore(self.interrupt_state);
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    static FIRST: IrqSpinLock<usize> = IrqSpinLock::new(0);
    static SECOND: IrqSpinLock<usize> = IrqSpinLock::new(0);

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
}
