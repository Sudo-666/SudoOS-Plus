use core::{
    cell::UnsafeCell,
    hint::spin_loop,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

/// 简单自旋锁。
///
/// 当前不是 IRQ-safe 锁。中断子系统建立后，需要再提供：
///
/// - IrqSpinLock；
/// - 保存/恢复本地中断状态；
/// - 锁顺序检查。
pub struct SpinLock<T: ?Sized> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }
}

impl<T: ?Sized> SpinLock<T> {
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        loop {
            if self
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return SpinLockGuard {
                    lock: self,
                    _not_send: PhantomData,
                };
            }

            /*
             * 避免在锁持续被占用时不断写 cache line。
             */
            while self.locked.load(Ordering::Relaxed) {
                spin_loop();
            }
        }
    }

    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| SpinLockGuard {
                lock: self,
                _not_send: PhantomData,
            })
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

/*
 * 被保护的值只要可以在线程/CPU 间移动，就可以通过 SpinLock
 * 被安全共享。
 */
// SAFETY: SpinLock 只通过 guard 暴露内部值，guard 持有期间具备独占访问。
unsafe impl<T: ?Sized + Send> Send for SpinLock<T> {}

// SAFETY: 共享 SpinLock 是安全的，因为内部可变性由原子锁状态串行化。
unsafe impl<T: ?Sized + Send> Sync for SpinLock<T> {}

#[must_use = "dropping the guard immediately unlocks the spin lock"]
pub struct SpinLockGuard<'a, T: ?Sized> {
    lock: &'a SpinLock<T>,

    _not_send: PhantomData<*mut ()>,
}

impl<T: ?Sized> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        /*
         * SAFETY:
         *
         * guard 存活期间锁处于独占状态。
         */
        unsafe { &*self.lock.value.get() }
    }
}

impl<T: ?Sized> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        /*
         * SAFETY:
         *
         * guard 是锁保护值的唯一可变访问路径。
         */
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T: ?Sized> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.unlock();
    }
}
