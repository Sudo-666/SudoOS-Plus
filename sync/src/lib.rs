#![no_std]

mod spin_lock;

pub use spin_lock::{SpinLock, SpinLockGuard};
