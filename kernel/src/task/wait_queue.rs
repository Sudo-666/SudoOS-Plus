use core::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
};

use super::{MAX_TASKS, TaskId};

static NEXT_WAIT_CHANNEL: AtomicUsize = AtomicUsize::new(1);
const COMPLETION_ALL: usize = usize::MAX / 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WaitEntryState {
    NotQueued,
    Queued,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WaitEntry {
    task: Option<TaskId>,
    state: WaitEntryState,
    exclusive: bool,
}

impl WaitEntry {
    const EMPTY: Self = Self {
        task: None,
        state: WaitEntryState::NotQueued,
        exclusive: true,
    };
}

struct WaitList {
    entries: [WaitEntry; MAX_TASKS],
    count: usize,
}

impl WaitList {
    const fn new() -> Self {
        Self {
            entries: [WaitEntry::EMPTY; MAX_TASKS],
            count: 0,
        }
    }

    fn enqueue(&mut self, task: TaskId, exclusive: bool) {
        assert!(
            !self
                .entries
                .iter()
                .any(|entry| entry.task == Some(task) && entry.state == WaitEntryState::Queued),
            "task was queued twice on the same wait queue: {task:?}",
        );

        let slot = self
            .entries
            .iter_mut()
            .find(|entry| entry.state == WaitEntryState::NotQueued)
            .expect("wait queue capacity exhausted");

        *slot = WaitEntry {
            task: Some(task),
            state: WaitEntryState::Queued,
            exclusive,
        };
        self.count = self.count.checked_add(1).expect("waiter count overflowed");
    }

    fn claim(&mut self, maximum: usize) -> ClaimedWaiters {
        assert!(maximum != 0, "wake limit must be non-zero");

        let mut claimed = ClaimedWaiters::empty();

        for entry in &mut self.entries {
            if claimed.count == maximum {
                break;
            }

            if entry.state != WaitEntryState::Queued {
                continue;
            }

            let task = entry.task.expect("queued wait entry lost its task");
            claimed.tasks[claimed.count] = Some(task);
            claimed.count += 1;

            let _exclusive = entry.exclusive;
            *entry = WaitEntry::EMPTY;
            self.count = self.count.checked_sub(1).expect("waiter count underflowed");
        }

        claimed
    }

    fn waiter_count(&self) -> usize {
        self.count
    }
}

pub(super) struct ClaimedWaiters {
    pub(super) tasks: [Option<TaskId>; MAX_TASKS],
    pub(super) count: usize,
}

impl ClaimedWaiters {
    const fn empty() -> Self {
        Self {
            tasks: [None; MAX_TASKS],
            count: 0,
        }
    }
}

/// A scheduler wait queue with an explicit waiter list.
///
/// The condition protected by a wait queue must be published before calling
/// `wake_one` or `wake_all`. `wait_until` rechecks the condition while the
/// scheduler lock is held, then queues the current task before switching out.
pub struct WaitQueue {
    channel: AtomicUsize,
    waiters: IrqSpinLock<WaitList>,
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

    pub(super) fn enqueue_current(&self, task: TaskId) {
        self.waiters.lock().enqueue(task, true);
    }

    pub(super) fn claim_waiters(&self, maximum: usize) -> ClaimedWaiters {
        self.waiters.lock().claim(maximum)
    }

    fn waiter_count_inner(&self) -> usize {
        self.waiters.lock().waiter_count()
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

    /// Consumes one completion token without blocking.
    ///
    /// `complete_all()` leaves the completion permanently signalled until an
    /// externally quiescent caller invokes `reinit()`.
    pub fn try_wait(&self) -> bool {
        loop {
            let done = self.done.load(Ordering::Acquire);
            if done == 0 {
                return false;
            }
            if done == COMPLETION_ALL {
                return true;
            }
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
                    done.checked_add(1)
                }
            })
            .expect("completion counter overflowed");
        self.waiters.wake_one();
    }

    pub fn complete_all(&self) {
        self.done.store(COMPLETION_ALL, Ordering::Release);
        self.waiters.wake_all();
    }

    /// Resets a completion after all users of the previous generation have
    /// quiesced. Reinitialising concurrently with wait/complete is invalid.
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
    crate::println!("  counted completion token : verified");
    crate::println!("  complete-all generation  : verified");
    crate::println!("  quiescent reinitialise   : verified");
}
