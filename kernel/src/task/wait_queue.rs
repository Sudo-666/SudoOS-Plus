use core::{
    mem::size_of,
    sync::atomic::{AtomicU8, AtomicUsize, Ordering},
    time::Duration,
};

use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
    time::MonotonicInstant,
};

use super::TaskId;

static NEXT_WAIT_CHANNEL: AtomicUsize = AtomicUsize::new(1);
const COMPLETION_ALL: usize = usize::MAX / 2;
const TIMEOUT_WAITING: u8 = 0;
const TIMEOUT_FIRED: u8 = 1;
const TIMEOUT_CANCELLED: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitOutcome {
    Satisfied,
    TimedOut,
}

/// Compact queue head. Waiter linkage lives in `Task`, exactly as Linux keeps
/// the queue head small and stores a list node in each wait entry.
///
/// All mutations happen while holding `SCHEDULER` followed by this queue's
/// WaitQueue-rank lock. This makes task state and queue ownership one atomic
/// scheduler transaction without allocating in IRQ context.
pub(super) struct WaitList {
    pub(super) head: Option<TaskId>,
    pub(super) tail: Option<TaskId>,
    pub(super) count: usize,
}

impl WaitList {
    const fn new() -> Self {
        Self {
            head: None,
            tail: None,
            count: 0,
        }
    }
}

const _: () = {
    // Never allow a task-count-sized array to creep back into the queue head.
    assert!(size_of::<WaitList>() <= 6 * size_of::<usize>());
};

struct TimeoutContext {
    state: AtomicU8,
    queue: *const WaitQueue,
    task: TaskId,
}

fn timeout_callback(argument: usize) {
    // SAFETY: the waiter keeps this stack object alive and calls
    // `cancel_sync()` before returning from the timeout API.
    let timeout = unsafe { &*(argument as *const TimeoutContext) };
    if timeout
        .state
        .compare_exchange(
            TIMEOUT_WAITING,
            TIMEOUT_FIRED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }
    // SAFETY: queue lifetime is covered by the same cancel_sync contract.
    let queue = unsafe { &*timeout.queue };
    super::wake_task_on_queue(queue, timeout.task);
}

/// A compact scheduler wait-queue head.
///
/// The queue owns only a head/tail/count triple. Each blocked task contributes
/// its own intrusive links, so embedding a WaitQueue or Completion in another
/// kernel object is constant-size and cannot consume kilobytes of kernel stack.
pub struct WaitQueue {
    channel: AtomicUsize,
    pub(super) waiters: IrqSpinLock<WaitList>,
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            channel: AtomicUsize::new(0),
            waiters: IrqSpinLock::new_with_class(
                WaitList::new(),
                LockClass::new("task_wait_queue", LockRank::WaitQueue, 1),
            ),
        }
    }

    pub fn wait_until<F>(&self, condition: F)
    where
        F: Fn() -> bool,
    {
        loop {
            if condition() {
                return;
            }
            let blocked = super::block_current_on_if(self, || !condition());
            if !blocked {
                return;
            }
        }
    }

    pub fn wait_timeout<F>(&self, timeout: Duration, condition: F) -> WaitOutcome
    where
        F: Fn() -> bool,
    {
        self.wait_until_deadline(crate::time::deadline_after(timeout), condition)
    }

    pub fn wait_until_deadline<F>(&self, deadline: MonotonicInstant, condition: F) -> WaitOutcome
    where
        F: Fn() -> bool,
    {
        crate::context::assert_task_context();
        crate::context::assert_interrupts_enabled();
        if condition() {
            return WaitOutcome::Satisfied;
        }
        if crate::time::deadline_reached(crate::time::now(), deadline) {
            return if condition() {
                WaitOutcome::Satisfied
            } else {
                WaitOutcome::TimedOut
            };
        }

        let timeout = TimeoutContext {
            state: AtomicU8::new(TIMEOUT_WAITING),
            queue: self as *const WaitQueue,
            task: super::current_task_id(),
        };
        let handle = crate::timer::arm_at(
            deadline,
            timeout_callback,
            core::ptr::addr_of!(timeout) as usize,
        )
        .unwrap_or_else(|error| panic!("unable to allocate wait timeout: {error:?}"));

        let outcome = loop {
            if condition() {
                break WaitOutcome::Satisfied;
            }
            if timeout.state.load(Ordering::Acquire) == TIMEOUT_FIRED {
                break if condition() {
                    WaitOutcome::Satisfied
                } else {
                    WaitOutcome::TimedOut
                };
            }
            let _ = super::block_current_on_if(self, || {
                !condition() && timeout.state.load(Ordering::Acquire) == TIMEOUT_WAITING
            });
        };

        if outcome == WaitOutcome::Satisfied {
            let _ = timeout.state.compare_exchange(
                TIMEOUT_WAITING,
                TIMEOUT_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        let _ = crate::timer::cancel_sync(handle);

        // The protected condition wins a boundary race, matching Linux's
        // wait-event timeout rule: true-at-expiry is success, not a spurious
        // timeout.
        if condition() {
            WaitOutcome::Satisfied
        } else if timeout.state.load(Ordering::Acquire) == TIMEOUT_FIRED {
            WaitOutcome::TimedOut
        } else {
            outcome
        }
    }

    pub fn wake_one(&self) -> usize {
        super::wake_queue(self, 1)
    }

    pub fn wake_all(&self) -> usize {
        super::wake_queue(self, super::MAX_TASKS)
    }

    #[cfg(debug_assertions)]
    pub fn waiter_count(&self) -> usize {
        self.waiter_count_inner()
    }

    #[cfg(debug_assertions)]
    pub(super) fn debug_state(&self) -> super::WaiterDebugState {
        super::waiter_debug_state(self.channel())
    }

    pub(super) fn channel(&self) -> usize {
        let current = self.channel.load(Ordering::Acquire);
        if current != 0 {
            return current;
        }

        let allocated = NEXT_WAIT_CHANNEL.fetch_add(1, Ordering::AcqRel);
        assert!(
            allocated != 0 && allocated != usize::MAX,
            "wait-channel identifier space exhausted",
        );
        match self
            .channel
            .compare_exchange(0, allocated, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => allocated,
            Err(existing) => existing,
        }
    }

    fn waiter_count_inner(&self) -> usize {
        self.waiters.lock().count
    }

    fn assert_empty(&self, operation: &str) {
        let waiters = self.waiter_count_inner();
        assert_eq!(
            waiters,
            0,
            "{operation} with waiters still queued: channel={} waiters={waiters}",
            self.channel.load(Ordering::Acquire),
        );
    }
}

impl Drop for WaitQueue {
    fn drop(&mut self) {
        self.assert_empty("wait queue dropped");
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Completion {
    done: AtomicUsize,
    waiters: WaitQueue,
}

impl Completion {
    pub const fn new() -> Self {
        Self {
            done: AtomicUsize::new(0),
            waiters: WaitQueue::new(),
        }
    }

    pub fn wait(&self) {
        loop {
            if self.try_wait() {
                return;
            }
            self.waiters
                .wait_until(|| self.done.load(Ordering::Acquire) != 0);
        }
    }

    pub fn wait_timeout(&self, timeout: Duration) -> WaitOutcome {
        let deadline = crate::time::deadline_after(timeout);
        loop {
            if self.try_wait() {
                return WaitOutcome::Satisfied;
            }
            match self
                .waiters
                .wait_until_deadline(deadline, || self.done.load(Ordering::Acquire) != 0)
            {
                WaitOutcome::Satisfied => {}
                WaitOutcome::TimedOut => {
                    return if self.try_wait() {
                        WaitOutcome::Satisfied
                    } else {
                        WaitOutcome::TimedOut
                    };
                }
            }
        }
    }

    pub fn try_wait(&self) -> bool {
        loop {
            let done = self.done.load(Ordering::Acquire);
            if done == 0 {
                return false;
            }
            if done == COMPLETION_ALL {
                return true;
            }
            assert!(
                done < COMPLETION_ALL,
                "completion counter entered the reserved complete-all range",
            );
            if self
                .done
                .compare_exchange_weak(done, done - 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return true;
            }
        }
    }

    pub fn complete(&self) {
        self.done
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |done| {
                if done == COMPLETION_ALL {
                    Some(COMPLETION_ALL)
                } else {
                    done.checked_add(1).filter(|next| *next < COMPLETION_ALL)
                }
            })
            .expect("completion counter overflowed");
        self.waiters.wake_one();
    }

    pub fn complete_all(&self) {
        self.done.store(COMPLETION_ALL, Ordering::Release);
        self.waiters.wake_all();
    }

    pub fn reinit(&self) {
        self.waiters.assert_empty("completion reinitialised");
        self.done.store(0, Ordering::Release);
    }

    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire) != 0
    }
}

impl Default for Completion {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = {
    // These caps are deliberately generous enough for lockdep metadata while
    // still making a regression to O(MAX_TASKS) storage fail at compile time.
    assert!(size_of::<WaitQueue>() <= 256);
    assert!(size_of::<Completion>() <= 320);
};

#[cfg(debug_assertions)]
pub(super) fn verify_local() {
    let completion = Completion::new();
    assert!(!completion.is_done());
    assert!(!completion.try_wait());
    completion.complete();
    assert!(completion.is_done());
    assert!(completion.try_wait());
    assert!(!completion.is_done());
    assert!(!completion.try_wait());
    completion.complete_all();
    assert!(completion.try_wait());
    assert!(completion.try_wait());
    assert!(completion.is_done());
    completion.reinit();
    assert!(!completion.is_done());
    assert!(!completion.try_wait());

    crate::println!("wait queue/completion invariant test:");
    crate::println!("  compact intrusive head   : verified");
    crate::println!("  counted completion token : verified");
    crate::println!("  complete-all generation  : verified");
    crate::println!("  quiescent reinitialise   : verified");
}
