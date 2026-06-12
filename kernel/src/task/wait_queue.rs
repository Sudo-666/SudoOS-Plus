use core::sync::atomic::{AtomicUsize, Ordering};

static NEXT_WAIT_CHANNEL: AtomicUsize = AtomicUsize::new(1);

/// A scheduler wait channel.
///
/// The condition protected by a wait queue must be published before calling
/// `wake_one` or `wake_all`. `wait_until` rechecks the condition while the
/// scheduler lock is held, and the task state machine records a wakeup that
/// races with the final context switch, preventing lost wakeups.
pub struct WaitQueue {
    channel: AtomicUsize,
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            channel: AtomicUsize::new(0),
        }
    }

    pub fn wait_until<F>(&self, condition: F)
    where
        F: Fn() -> bool,
    {
        let channel = self.channel();

        loop {
            if condition() {
                return;
            }

            let blocked = super::block_current_on_if(channel, || !condition());
            if !blocked {
                return;
            }
        }
    }

    pub fn wake_one(&self) -> usize {
        super::wake_channel(self.channel(), 1)
    }

    pub fn wake_all(&self) -> usize {
        super::wake_channel(self.channel(), super::MAX_TASKS)
    }

    #[cfg(debug_assertions)]
    pub fn waiter_count(&self) -> usize {
        super::waiter_count(self.channel())
    }

    fn channel(&self) -> usize {
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
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}
