use core::{
    hint::spin_loop,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
    smp::{CpuId, MAX_CPUS},
    task::WaitQueue,
    time::MonotonicInstant,
};

const TIMERS_PER_CPU: usize = 128;
const MAX_CALLBACKS_PER_INTERRUPT: usize = 32;

pub type TimerCallback = fn(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerError {
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerHandle {
    owner: CpuId,
    slot: u16,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TimerState {
    Free,
    Armed,
    Firing,
}

#[derive(Clone, Copy)]
struct TimerSlot {
    state: TimerState,
    generation: u64,
    deadline: MonotonicInstant,
    callback: Option<TimerCallback>,
    argument: usize,
}

impl TimerSlot {
    const EMPTY: Self = Self {
        state: TimerState::Free,
        generation: 0,
        deadline: MonotonicInstant::from_cycles(0),
        callback: None,
        argument: 0,
    };
}

#[derive(Clone, Copy)]
struct FiringTimer {
    slot: usize,
    generation: u64,
    callback: TimerCallback,
    argument: usize,
}

struct TimerQueue {
    slots: [TimerSlot; TIMERS_PER_CPU],
    order: [u16; TIMERS_PER_CPU],
    len: usize,
}

impl TimerQueue {
    const fn new() -> Self {
        Self {
            slots: [TimerSlot::EMPTY; TIMERS_PER_CPU],
            order: [0; TIMERS_PER_CPU],
            len: 0,
        }
    }

    fn arm(
        &mut self,
        owner: CpuId,
        deadline: MonotonicInstant,
        callback: TimerCallback,
        argument: usize,
    ) -> Result<TimerHandle, TimerError> {
        let slot = self
            .slots
            .iter()
            .position(|slot| slot.state == TimerState::Free)
            .ok_or(TimerError::Capacity)?;
        let generation = self.slots[slot].generation.wrapping_add(1).max(1);
        self.slots[slot] = TimerSlot {
            state: TimerState::Armed,
            generation,
            deadline,
            callback: Some(callback),
            argument,
        };
        self.insert_ordered(slot);
        Ok(TimerHandle {
            owner,
            slot: u16::try_from(slot).expect("timer slot index exceeds u16"),
            generation,
        })
    }

    fn insert_ordered(&mut self, slot: usize) {
        assert!(self.len < TIMERS_PER_CPU, "timer order array overflowed");
        let deadline = self.slots[slot].deadline;
        let mut position = self.len;
        while position != 0 {
            let previous = usize::from(self.order[position - 1]);
            if !crate::time::instant_is_before(deadline, self.slots[previous].deadline) {
                break;
            }
            self.order[position] = self.order[position - 1];
            position -= 1;
        }
        self.order[position] = u16::try_from(slot).expect("timer slot index exceeds u16");
        self.len += 1;
    }

    fn earliest(&self) -> Option<MonotonicInstant> {
        (self.len != 0).then(|| self.slots[usize::from(self.order[0])].deadline)
    }

    fn pop_due(&mut self, now: MonotonicInstant) -> Option<FiringTimer> {
        if self.len == 0 {
            return None;
        }
        let slot = usize::from(self.order[0]);
        if !crate::time::deadline_reached(now, self.slots[slot].deadline) {
            return None;
        }
        self.remove_order_position(0);
        let timer = &mut self.slots[slot];
        assert_eq!(timer.state, TimerState::Armed);
        timer.state = TimerState::Firing;
        Some(FiringTimer {
            slot,
            generation: timer.generation,
            callback: timer.callback.expect("armed timer lost its callback"),
            argument: timer.argument,
        })
    }

    fn complete_firing(&mut self, firing: FiringTimer) {
        let timer = &mut self.slots[firing.slot];
        assert_eq!(timer.generation, firing.generation);
        assert_eq!(timer.state, TimerState::Firing);
        timer.state = TimerState::Free;
        timer.callback = None;
        timer.argument = 0;
    }

    fn cancel(&mut self, handle: TimerHandle) -> CancelResult {
        let slot = usize::from(handle.slot);
        let Some(timer) = self.slots.get(slot) else {
            return CancelResult::Stale;
        };
        if timer.generation != handle.generation {
            return CancelResult::Stale;
        }
        match timer.state {
            TimerState::Free => CancelResult::Stale,
            TimerState::Firing => CancelResult::Firing,
            TimerState::Armed => {
                let position = self.order[..self.len]
                    .iter()
                    .position(|entry| usize::from(*entry) == slot)
                    .expect("armed timer is absent from deadline order");
                self.remove_order_position(position);
                let timer = &mut self.slots[slot];
                timer.state = TimerState::Free;
                timer.callback = None;
                timer.argument = 0;
                CancelResult::Cancelled
            }
        }
    }

    fn remove_order_position(&mut self, position: usize) {
        assert!(position < self.len, "timer order removal is out of bounds");
        for index in position..self.len - 1 {
            self.order[index] = self.order[index + 1];
        }
        self.len -= 1;
        self.order[self.len] = 0;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CancelResult {
    Cancelled,
    Firing,
    Stale,
}

struct TimerBase {
    queue: IrqSpinLock<TimerQueue>,
}

impl TimerBase {
    const fn new() -> Self {
        Self {
            queue: IrqSpinLock::new_with_class(
                TimerQueue::new(),
                LockClass::new("timer_base", LockRank::Timer, 0),
            ),
        }
    }
}

static TIMER_BASES: [TimerBase; MAX_CPUS] = [const { TimerBase::new() }; MAX_CPUS];
static INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn initialize() {
    assert!(
        crate::arch::interrupt::are_disabled(),
        "timer runtime initialization requires local interrupts disabled",
    );
    assert!(
        INITIALIZED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok(),
        "timer runtime was initialized more than once",
    );
    crate::println!("timer runtime:");
    crate::println!("  bases            : per-CPU");
    crate::println!("  capacity         : {} timers/CPU", TIMERS_PER_CPU);
    crate::println!("  callback context : hardirq, timer lock released");
    crate::println!("  cancel contract  : synchronous");
}

/// Arm a timer on the current CPU.
///
/// The callback runs in hard-IRQ context after the timer-base lock is released.
/// It must not sleep and must perform bounded work.
pub fn arm_at(
    deadline: MonotonicInstant,
    callback: TimerCallback,
    argument: usize,
) -> Result<TimerHandle, TimerError> {
    assert_initialized();
    // Queue publication and local clockevent programming are one IRQ-atomic
    // transaction. Otherwise an interrupt between lock release and reprogram
    // could install an older deadline over a callback's newer one.
    let _interrupt_guard = crate::context::IrqSaveGuard::new();
    let owner = crate::smp::current_cpu_id();
    let base = &TIMER_BASES[owner.get()];
    let (handle, earliest) = {
        let mut queue = base.queue.lock();
        let handle = queue.arm(owner, deadline, callback, argument)?;
        (handle, queue.earliest())
    };
    crate::time::reprogram_local(earliest);
    Ok(handle)
}

pub fn arm_after(
    duration: Duration,
    callback: TimerCallback,
    argument: usize,
) -> Result<TimerHandle, TimerError> {
    arm_at(crate::time::deadline_after(duration), callback, argument)
}

/// Cancel a timer and wait until an already-running callback finishes.
///
/// This is the kernel's `del_timer_sync()`-style primitive. It must be called
/// from sleepable task context with local interrupts enabled. A callback must
/// never call `cancel_sync()` on its own handle.
pub fn cancel_sync(handle: TimerHandle) -> bool {
    assert_initialized();
    crate::context::might_sleep();
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();

    let base = &TIMER_BASES[handle.owner.get()];
    loop {
        // Keep local queue mutation and local hardware reprogramming IRQ-atomic,
        // but restore interrupts before waiting for a remote firing callback.
        let interrupt_guard = crate::context::IrqSaveGuard::new();
        let (result, earliest) = {
            let mut queue = base.queue.lock();
            let result = queue.cancel(handle);
            (result, queue.earliest())
        };
        if result == CancelResult::Cancelled && handle.owner == crate::smp::current_cpu_id() {
            crate::time::reprogram_local(earliest);
        }
        drop(interrupt_guard);

        match result {
            CancelResult::Cancelled => return true,
            CancelResult::Stale => return false,
            CancelResult::Firing => spin_loop(),
        }
    }
}

/// Run expired callbacks for the current CPU and return its next deadline.
///
/// Callback execution is deliberately budgeted. If more work remains, the
/// returned expired deadline causes the clockevent to retrigger immediately.
pub fn handle_interrupt(now: MonotonicInstant) -> Option<MonotonicInstant> {
    assert_initialized();
    let cpu = crate::smp::current_cpu_id();
    let base = &TIMER_BASES[cpu.get()];
    for _ in 0..MAX_CALLBACKS_PER_INTERRUPT {
        let firing = {
            let mut queue = base.queue.lock();
            queue.pop_due(now)
        };
        let Some(firing) = firing else {
            break;
        };

        (firing.callback)(firing.argument);

        let mut queue = base.queue.lock();
        queue.complete_firing(firing);
    }
    base.queue.lock().earliest()
}

pub(crate) fn earliest_local() -> Option<MonotonicInstant> {
    assert_initialized();
    TIMER_BASES[crate::smp::current_cpu_id().get()]
        .queue
        .lock()
        .earliest()
}

struct SleepContext {
    complete: AtomicBool,
    waiters: WaitQueue,
}

fn sleep_callback(argument: usize) {
    // SAFETY: `sleep_until` keeps this stack object alive and executes
    // `cancel_sync` before returning, so the callback cannot outlive it.
    let context = unsafe { &*(argument as *const SleepContext) };
    context.complete.store(true, Ordering::Release);
    context.waiters.wake_one();
}

pub fn sleep(duration: Duration) {
    if duration.is_zero() {
        return;
    }
    sleep_until(crate::time::deadline_after(duration));
}

pub fn sleep_until(deadline: MonotonicInstant) {
    assert_initialized();
    crate::context::might_sleep();
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();
    if crate::time::deadline_reached(crate::time::now(), deadline) {
        return;
    }

    let context = SleepContext {
        complete: AtomicBool::new(false),
        waiters: WaitQueue::new(),
    };
    let handle = arm_at(
        deadline,
        sleep_callback,
        core::ptr::addr_of!(context) as usize,
    )
    .unwrap_or_else(|error| panic!("unable to allocate sleep timer: {error:?}"));
    context
        .waiters
        .wait_until(|| context.complete.load(Ordering::Acquire));
    let _ = cancel_sync(handle);
}

fn assert_initialized() {
    assert!(
        INITIALIZED.load(Ordering::Acquire),
        "timer runtime used before initialization",
    );
}

#[cfg(debug_assertions)]
fn verify_irq_nesting_order() {
    static OUTER: crate::tracked_spin::TrackedSpinLock<()> =
        crate::tracked_spin::TrackedSpinLock::new_with_class(
            (),
            LockClass::new("timer_irq_nesting_probe", LockRank::CrossCpu, 63),
        );

    // Model the real interrupt edge which exposed the LoongArch failure:
    // task context holds an IRQ-enabled cross-CPU serializer, then a timer
    // hardirq enters and takes the IRQ-safe timer-base lock. The reverse edge
    // remains forbidden by the rank table.
    let outer = OUTER.lock();
    assert!(crate::arch::interrupt::are_enabled());
    {
        let queue = TIMER_BASES[crate::smp::current_cpu_id().get()].queue.lock();
        assert!(crate::arch::interrupt::are_disabled());
        drop(queue);
    }
    assert!(crate::arch::interrupt::are_enabled());
    drop(outer);
}

#[cfg(debug_assertions)]
mod verify {
    use alloc::vec::Vec;
    use core::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use super::*;
    use crate::task::{Completion, WaitOutcome};

    struct OrderContext {
        sequence: AtomicUsize,
        complete: Completion,
    }

    struct OrderArgument {
        context: *const OrderContext,
        expected: usize,
    }

    fn order_callback(argument: usize) {
        // SAFETY: the verifier synchronously cancels both handles before its
        // stack arguments leave scope.
        let argument = unsafe { &*(argument as *const OrderArgument) };
        let context = unsafe { &*argument.context };
        let observed = context.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        assert_eq!(observed, argument.expected, "timer deadline order violated");
        context.complete.complete();
    }

    fn increment_callback(argument: usize) {
        // SAFETY: every verifier path calls cancel_sync before the referenced
        // counter leaves scope.
        let counter = unsafe { &*(argument as *const AtomicUsize) };
        counter.fetch_add(1, Ordering::AcqRel);
    }

    // Keep every verifier case in a separate, non-inlined frame. Kernel task
    // stacks intentionally remain small and guard-paged; collecting several
    // WaitQueue/Completion objects in one debug worker would create a frame
    // larger than the 16-KiB stack before the first test instruction executes.
    #[inline(never)]
    fn verify_deadline_ordering() {
        let order = OrderContext {
            sequence: AtomicUsize::new(0),
            complete: Completion::new(),
        };
        let later_argument = OrderArgument {
            context: core::ptr::addr_of!(order),
            expected: 2,
        };
        let earlier_argument = OrderArgument {
            context: core::ptr::addr_of!(order),
            expected: 1,
        };
        let later = arm_after(
            Duration::from_millis(20),
            order_callback,
            core::ptr::addr_of!(later_argument) as usize,
        )
        .expect("unable to arm later verifier timer");
        let earlier = arm_after(
            Duration::from_millis(5),
            order_callback,
            core::ptr::addr_of!(earlier_argument) as usize,
        )
        .expect("unable to arm earlier verifier timer");
        order.complete.wait();
        order.complete.wait();
        let _ = cancel_sync(earlier);
        let _ = cancel_sync(later);
        assert_eq!(order.sequence.load(Ordering::Acquire), 2);
    }

    #[inline(never)]
    fn verify_synchronous_cancel() {
        let cancelled_callbacks = AtomicUsize::new(0);
        let cancelled = arm_after(
            Duration::from_secs(1),
            increment_callback,
            core::ptr::addr_of!(cancelled_callbacks) as usize,
        )
        .expect("unable to arm cancellation verifier timer");
        assert!(cancel_sync(cancelled));
        sleep(Duration::from_millis(10));
        assert_eq!(cancelled_callbacks.load(Ordering::Acquire), 0);
    }

    #[inline(never)]
    fn verify_short_sleep() {
        let before = crate::time::now();
        sleep(Duration::from_millis(10));
        let after = crate::time::now();
        assert!(crate::time::instant_is_after(after, before));
        assert!(after.duration_since(before) >= Duration::from_millis(10));
    }

    #[inline(never)]
    fn verify_long_sleep() {
        let before = crate::time::now();
        sleep(Duration::from_secs(1));
        let after = crate::time::now();
        assert!(after.duration_since(before) >= Duration::from_secs(1));
    }

    #[inline(never)]
    fn verify_wait_queue_timeout() {
        let waiters = WaitQueue::new();
        assert_eq!(
            waiters.wait_timeout(Duration::from_millis(5), || false),
            WaitOutcome::TimedOut,
        );
    }

    #[inline(never)]
    fn verify_completion_timeout() {
        let completion = Completion::new();
        assert_eq!(
            completion.wait_timeout(Duration::from_millis(5)),
            WaitOutcome::TimedOut,
        );
    }

    #[inline(never)]
    fn verify_slot_reclamation() {
        let callback_count = AtomicUsize::new(0);
        // This is verifier bookkeeping rather than scheduler state. Put it on
        // the heap so TIMERS_PER_CPU does not directly inflate a task frame.
        let mut handles = Vec::with_capacity(TIMERS_PER_CPU);
        for _ in 0..TIMERS_PER_CPU {
            handles.push(
                arm_after(
                    Duration::from_secs(5),
                    increment_callback,
                    core::ptr::addr_of!(callback_count) as usize,
                )
                .expect("timer slot reclamation setup failed"),
            );
        }
        for handle in handles {
            assert!(cancel_sync(handle));
        }
        assert_eq!(callback_count.load(Ordering::Acquire), 0);
    }

    pub(super) fn worker() {
        crate::context::assert_task_context();
        crate::context::assert_interrupts_enabled();

        super::verify_irq_nesting_order();

        verify_deadline_ordering();
        verify_synchronous_cancel();
        verify_short_sleep();
        verify_long_sleep();
        verify_wait_queue_timeout();
        verify_completion_timeout();
        verify_slot_reclamation();

        crate::println!("timer runtime test:");
        crate::println!("  deadline ordering : verified");
        crate::println!("  software deadline : verified");
        crate::println!("  synchronous cancel: verified");
        crate::println!("  kernel sleep      : verified");
        crate::println!("  wait timeout      : verified");
        crate::println!("  slot reclamation  : verified");
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    // wait_timeout(), Completion::wait() and sleep() are blocking APIs.  Keep
    // the boot CPU's idle task as a launcher only and execute all cases on a
    // normal guarded-stack kernel thread.
    crate::task::run_verifier_thread(verify::worker);
}
