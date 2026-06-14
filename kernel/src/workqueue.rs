use core::{
    sync::atomic::{AtomicU8, AtomicU32, AtomicUsize, Ordering},
    time::Duration,
};

use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
    smp::{CpuId, MAX_CPUS},
    task::WaitQueue,
    timer::TimerHandle,
};

const WORK_SLOTS_PER_CPU: usize = 128;
const WORKERS_PER_CPU: usize = 2;
const TOKEN_SLOT_BITS: usize = 8;
const TOKEN_CPU_BITS: usize = 8;
const TOKEN_GENERATION_SHIFT: usize = TOKEN_SLOT_BITS + TOKEN_CPU_BITS;
const TOKEN_SLOT_MASK: usize = (1 << TOKEN_SLOT_BITS) - 1;
const TOKEN_CPU_MASK: usize = (1 << TOKEN_CPU_BITS) - 1;
const GENERATION_HALF_RANGE: u32 = 1_u32 << 31;

const _: () = {
    assert!(usize::BITS >= 64);
    assert!(WORK_SLOTS_PER_CPU <= (1 << TOKEN_SLOT_BITS));
    assert!(MAX_CPUS <= (1 << TOKEN_CPU_BITS));
};

pub type WorkCallback = fn(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkError {
    Capacity,
    InvalidCpu,
    Timer(crate::timer::TimerError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkHandle {
    owner: CpuId,
    slot: u16,
    generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkState {
    Free,
    Arming,
    Delayed,
    Pending,
    Running,
    Cancelling,
}

#[derive(Clone, Copy)]
struct WorkSlot {
    state: WorkState,
    generation: u32,
    callback: Option<WorkCallback>,
    argument: usize,
    timer: Option<TimerHandle>,
}

impl WorkSlot {
    const EMPTY: Self = Self {
        state: WorkState::Free,
        generation: 0,
        callback: None,
        argument: 0,
        timer: None,
    };
}

#[derive(Clone, Copy)]
struct RunningWork {
    slot: usize,
    generation: u32,
    callback: WorkCallback,
    argument: usize,
}

struct WorkQueueState {
    slots: [WorkSlot; WORK_SLOTS_PER_CPU],
    order: [u16; WORK_SLOTS_PER_CPU],
    len: usize,
}

impl WorkQueueState {
    const fn new() -> Self {
        Self {
            slots: [WorkSlot::EMPTY; WORK_SLOTS_PER_CPU],
            order: [0; WORK_SLOTS_PER_CPU],
            len: 0,
        }
    }

    fn allocate(
        &mut self,
        callback: WorkCallback,
        argument: usize,
        state: WorkState,
    ) -> Result<(usize, u32), WorkError> {
        assert!(matches!(state, WorkState::Arming | WorkState::Pending));
        let slot = self
            .slots
            .iter()
            .position(|slot| slot.state == WorkState::Free)
            .ok_or(WorkError::Capacity)?;
        let generation = self.slots[slot].generation.wrapping_add(1).max(1);
        self.slots[slot] = WorkSlot {
            state,
            generation,
            callback: Some(callback),
            argument,
            timer: None,
        };
        if state == WorkState::Pending {
            self.push_pending(slot);
        }
        Ok((slot, generation))
    }

    fn push_pending(&mut self, slot: usize) {
        assert!(
            self.len < WORK_SLOTS_PER_CPU,
            "workqueue pending ring overflowed"
        );
        assert_eq!(self.slots[slot].state, WorkState::Pending);
        assert!(
            !self.order[..self.len]
                .iter()
                .any(|entry| usize::from(*entry) == slot),
            "work item was queued twice",
        );
        self.order[self.len] = u16::try_from(slot).expect("work slot exceeds u16");
        self.len += 1;
    }

    fn pop_pending(&mut self) -> Option<RunningWork> {
        if self.len == 0 {
            return None;
        }
        let slot = usize::from(self.order[0]);
        self.remove_order_position(0);
        let entry = &mut self.slots[slot];
        assert_eq!(entry.state, WorkState::Pending);
        entry.state = WorkState::Running;
        Some(RunningWork {
            slot,
            generation: entry.generation,
            callback: entry.callback.expect("pending work lost its callback"),
            argument: entry.argument,
        })
    }

    fn remove_pending(&mut self, slot: usize) {
        let position = self.order[..self.len]
            .iter()
            .position(|entry| usize::from(*entry) == slot)
            .expect("pending work is absent from the queue");
        self.remove_order_position(position);
    }

    fn remove_order_position(&mut self, position: usize) {
        assert!(position < self.len, "workqueue removal is out of bounds");
        for index in position..self.len - 1 {
            self.order[index] = self.order[index + 1];
        }
        self.len -= 1;
        self.order[self.len] = 0;
    }

    fn free(&mut self, slot: usize, generation: u32) {
        let entry = &mut self.slots[slot];
        assert_eq!(
            entry.generation, generation,
            "work generation changed while active"
        );
        assert_ne!(entry.state, WorkState::Free, "work slot freed twice");
        entry.state = WorkState::Free;
        entry.callback = None;
        entry.argument = 0;
        entry.timer = None;
    }
}

struct WorkBase {
    state: IrqSpinLock<WorkQueueState>,
    ready: WaitQueue,
    completed: WaitQueue,
    pending: AtomicUsize,
    completed_generation: [AtomicU32; WORK_SLOTS_PER_CPU],
}

impl WorkBase {
    const fn new() -> Self {
        Self {
            state: IrqSpinLock::new_with_class(
                WorkQueueState::new(),
                LockClass::new("workqueue_base", LockRank::WorkQueue, 0),
            ),
            ready: WaitQueue::new(),
            completed: WaitQueue::new(),
            pending: AtomicUsize::new(0),
            completed_generation: [const { AtomicU32::new(0) }; WORK_SLOTS_PER_CPU],
        }
    }

    fn increment_pending(&self) {
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .expect("workqueue pending counter overflowed");
    }

    fn decrement_pending(&self) {
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_sub(1)
            })
            .expect("workqueue pending counter underflowed");
    }

    fn wake_worker(&self) {
        self.ready.wake_one();
    }

    fn publish_completion(&self, slot: usize, generation: u32) {
        self.completed_generation[slot].store(generation, Ordering::Release);
        self.completed.wake_all();
    }

    fn is_complete(&self, handle: WorkHandle) -> bool {
        generation_reached(
            self.completed_generation[usize::from(handle.slot)].load(Ordering::Acquire),
            handle.generation,
        )
    }
}

static WORK_BASES: [WorkBase; MAX_CPUS] = [const { WorkBase::new() }; MAX_CPUS];
const INIT_UNINITIALIZED: u8 = 0;
const INIT_INITIALIZING: u8 = 1;
const INIT_READY: u8 = 2;
const NO_WORKER_TASK: usize = usize::MAX;

static INIT_STATE: AtomicU8 = AtomicU8::new(INIT_UNINITIALIZED);
static WORKER_TASKS: [[AtomicUsize; WORKERS_PER_CPU]; MAX_CPUS] =
    [const { [const { AtomicUsize::new(NO_WORKER_TASK) }; WORKERS_PER_CPU] }; MAX_CPUS];

pub fn initialize() {
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();
    assert!(
        INIT_STATE
            .compare_exchange(
                INIT_UNINITIALIZED,
                INIT_INITIALIZING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok(),
        "workqueue runtime was initialized more than once",
    );

    let discovered = crate::smp::discovered_cpu_count();
    for index in 0..discovered {
        let cpu = CpuId::new(index).expect("workqueue CPU exceeds MAX_CPUS");
        assert!(
            crate::smp::is_scheduler_active(cpu),
            "workqueue worker target is not scheduler-active",
        );
        for worker_index in 0..WORKERS_PER_CPU {
            let worker = crate::task::spawn_system_thread_on(worker_entry, cpu);
            assert_eq!(
                WORKER_TASKS[index][worker_index].swap(worker.raw(), Ordering::AcqRel),
                NO_WORKER_TASK,
                "workqueue worker task was published twice",
            );
        }
    }
    INIT_STATE.store(INIT_READY, Ordering::Release);

    crate::println!("workqueue runtime:");
    crate::println!(
        "  topology         : {} bounded workers per CPU",
        WORKERS_PER_CPU
    );
    crate::println!("  capacity         : {} work items/CPU", WORK_SLOTS_PER_CPU);
    crate::println!("  callback context : sleepable system thread");
    crate::println!("  delayed work     : timer publishes, worker executes");
    crate::println!("  cancel contract  : synchronous task-context teardown");
}

/// Queue immediate work on the current CPU.
///
/// This operation is allocation-free and IRQ-safe. The callback is never run in
/// hardirq context; a per-CPU system worker executes it with interrupts enabled.
pub fn queue(callback: WorkCallback, argument: usize) -> Result<WorkHandle, WorkError> {
    queue_on(crate::smp::current_cpu_id(), callback, argument)
}

/// Queue immediate work on a selected scheduler-active CPU.
pub fn queue_on(
    owner: CpuId,
    callback: WorkCallback,
    argument: usize,
) -> Result<WorkHandle, WorkError> {
    assert_initialized();
    if owner.get() >= crate::smp::discovered_cpu_count() || !crate::smp::is_scheduler_active(owner)
    {
        return Err(WorkError::InvalidCpu);
    }

    let base = &WORK_BASES[owner.get()];
    let (slot, generation) = {
        let mut state = base.state.lock();
        let allocated = state.allocate(callback, argument, WorkState::Pending)?;
        // Publish the atomic waiter predicate before the queue lock is released.
        // A running worker can otherwise observe the ring entry before the
        // pending count and underflow the mirror while consuming it.
        base.increment_pending();
        allocated
    };
    base.wake_worker();
    Ok(WorkHandle {
        owner,
        slot: u16::try_from(slot).expect("work slot exceeds u16"),
        generation,
    })
}

/// Queue delayed work on the current CPU.
///
/// The timer callback performs only the Delayed -> Pending publication. The
/// supplied callback executes later in the sleepable system-worker context.
pub fn queue_delayed(
    delay: Duration,
    callback: WorkCallback,
    argument: usize,
) -> Result<WorkHandle, WorkError> {
    assert_initialized();
    let interrupt_guard = crate::context::IrqSaveGuard::new();
    let owner = crate::smp::current_cpu_id();
    let base = &WORK_BASES[owner.get()];
    let (slot, generation) = {
        let mut state = base.state.lock();
        state.allocate(callback, argument, WorkState::Arming)?
    };
    let handle = WorkHandle {
        owner,
        slot: u16::try_from(slot).expect("work slot exceeds u16"),
        generation,
    };
    let token = encode_token(handle);

    let timer = match crate::timer::arm_after(delay, delayed_timer_callback, token) {
        Ok(timer) => timer,
        Err(error) => {
            {
                let mut state = base.state.lock();
                assert_eq!(state.slots[slot].generation, generation);
                assert_eq!(state.slots[slot].state, WorkState::Arming);
                state.free(slot, generation);
            }
            base.publish_completion(slot, generation);
            drop(interrupt_guard);
            return Err(WorkError::Timer(error));
        }
    };

    {
        let mut state = base.state.lock();
        let entry = &mut state.slots[slot];
        assert_eq!(entry.generation, generation);
        assert_eq!(entry.state, WorkState::Arming);
        entry.timer = Some(timer);
        entry.state = WorkState::Delayed;
    }
    drop(interrupt_guard);
    Ok(handle)
}

/// Wait until a work item is no longer pending, delayed, or running.
///
/// Synchronous waits from a workqueue callback are rejected because a callback
/// can otherwise wait for work owned by the same pinned worker and deadlock.
pub fn flush(handle: WorkHandle) {
    assert_sync_allowed();
    let base = base_for_handle(handle);
    base.completed.wait_until(|| base.is_complete(handle));
}

/// Cancel delayed or pending work and wait for a running callback to finish.
///
/// Returns `true` when the user callback was prevented from running. Returns
/// `false` for a stale handle or when the callback had already started.
pub fn cancel_sync(handle: WorkHandle) -> bool {
    assert_sync_allowed();
    let base = base_for_handle(handle);
    if base.is_complete(handle) {
        return false;
    }

    enum Action {
        Stale,
        CancelDelayed(TimerHandle),
        CancelPending,
        WaitRunning,
        WaitCancelling,
    }

    let slot = usize::from(handle.slot);
    let action = {
        let mut state = base.state.lock();
        if state.slots[slot].generation != handle.generation {
            Action::Stale
        } else {
            match state.slots[slot].state {
                WorkState::Free => Action::Stale,
                WorkState::Arming => {
                    panic!("published work handle remained in the arming state")
                }
                WorkState::Delayed => {
                    let timer = state.slots[slot]
                        .timer
                        .take()
                        .expect("delayed work lost its timer");
                    state.slots[slot].state = WorkState::Cancelling;
                    Action::CancelDelayed(timer)
                }
                WorkState::Pending => {
                    state.remove_pending(slot);
                    base.decrement_pending();
                    state.free(slot, handle.generation);
                    Action::CancelPending
                }
                WorkState::Running => Action::WaitRunning,
                WorkState::Cancelling => Action::WaitCancelling,
            }
        }
    };

    match action {
        Action::Stale => false,
        Action::CancelPending => {
            base.publish_completion(slot, handle.generation);
            true
        }
        Action::CancelDelayed(timer) => {
            // Whether cancellation removes an armed timer or waits for a
            // firing callback, cancel_sync guarantees that the timer-side
            // publication is quiescent before the slot is reclaimed.
            let _ = crate::timer::cancel_sync(timer);
            {
                let mut state = base.state.lock();
                assert_eq!(state.slots[slot].generation, handle.generation);
                assert_eq!(state.slots[slot].state, WorkState::Cancelling);
                state.free(slot, handle.generation);
            }
            base.publish_completion(slot, handle.generation);
            true
        }
        Action::WaitRunning | Action::WaitCancelling => {
            base.completed.wait_until(|| base.is_complete(handle));
            false
        }
    }
}

pub fn is_complete(handle: WorkHandle) -> bool {
    base_for_handle(handle).is_complete(handle)
}

fn delayed_timer_callback(argument: usize) {
    let handle = decode_token(argument);
    let base = &WORK_BASES[handle.owner.get()];
    let slot = usize::from(handle.slot);
    let queued = {
        let mut state = base.state.lock();
        if state.slots[slot].generation != handle.generation {
            false
        } else {
            match state.slots[slot].state {
                WorkState::Delayed => {
                    state.slots[slot].timer = None;
                    state.slots[slot].state = WorkState::Pending;
                    state.push_pending(slot);
                    // Keep the atomic waiter predicate and protected ring in one
                    // publication transaction. The wake itself stays lock-free.
                    base.increment_pending();
                    true
                }
                WorkState::Cancelling => {
                    state.slots[slot].timer = None;
                    false
                }
                other => panic!("delayed timer observed invalid work state: {other:?}"),
            }
        }
    };
    if queued {
        base.wake_worker();
    }
}

fn worker_main(expected_cpu: CpuId) -> ! {
    assert_eq!(
        crate::smp::current_cpu_id(),
        expected_cpu,
        "workqueue worker ran on the wrong CPU",
    );
    let base = &WORK_BASES[expected_cpu.get()];

    loop {
        base.ready
            .wait_until(|| base.pending.load(Ordering::Acquire) != 0);
        while let Some(work) = take_pending_work(base) {
            crate::context::assert_task_context();
            crate::context::assert_interrupts_enabled();
            (work.callback)(work.argument);

            {
                let mut state = base.state.lock();
                assert_eq!(state.slots[work.slot].generation, work.generation);
                assert_eq!(state.slots[work.slot].state, WorkState::Running);
                state.free(work.slot, work.generation);
            }
            base.publish_completion(work.slot, work.generation);
        }
    }
}

fn take_pending_work(base: &WorkBase) -> Option<RunningWork> {
    let mut state = base.state.lock();
    let work = state.pop_pending();
    if work.is_some() {
        base.decrement_pending();
    }
    work
}

fn assert_initialized() {
    assert_eq!(
        INIT_STATE.load(Ordering::Acquire),
        INIT_READY,
        "workqueue runtime used before initialization completed",
    );
}

fn assert_sync_allowed() {
    assert_initialized();
    crate::context::might_sleep();
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();
    let cpu = crate::smp::current_cpu_id();
    let current = crate::task::current_task_id().raw();
    assert!(
        WORKER_TASKS[cpu.get()]
            .iter()
            .all(|worker| worker.load(Ordering::Acquire) != current),
        "synchronous workqueue wait from a workqueue callback would deadlock",
    );
}

fn base_for_handle(handle: WorkHandle) -> &'static WorkBase {
    assert_initialized();
    assert!(
        handle.owner.get() < MAX_CPUS,
        "work handle owner is invalid"
    );
    assert!(
        usize::from(handle.slot) < WORK_SLOTS_PER_CPU,
        "work handle slot is invalid",
    );
    &WORK_BASES[handle.owner.get()]
}

fn generation_reached(completed: u32, target: u32) -> bool {
    completed == target || completed.wrapping_sub(target) < GENERATION_HALF_RANGE
}

fn encode_token(handle: WorkHandle) -> usize {
    (usize::try_from(handle.generation).expect("generation exceeds usize")
        << TOKEN_GENERATION_SHIFT)
        | (handle.owner.get() << TOKEN_SLOT_BITS)
        | usize::from(handle.slot)
}

fn decode_token(token: usize) -> WorkHandle {
    let slot = token & TOKEN_SLOT_MASK;
    let owner = (token >> TOKEN_SLOT_BITS) & TOKEN_CPU_MASK;
    let generation = u32::try_from(token >> TOKEN_GENERATION_SHIFT)
        .expect("delayed-work generation token overflowed");
    WorkHandle {
        owner: CpuId::new(owner).expect("delayed-work token CPU is invalid"),
        slot: u16::try_from(slot).expect("delayed-work token slot exceeds u16"),
        generation,
    }
}

fn worker_entry() {
    worker_main(crate::smp::current_cpu_id());
}

#[cfg(debug_assertions)]
mod verify {
    use alloc::{boxed::Box, vec::Vec};
    use core::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use super::*;

    struct Probe {
        hits: AtomicUsize,
        cpu_mask: AtomicUsize,
    }

    impl Probe {
        const fn new() -> Self {
            Self {
                hits: AtomicUsize::new(0),
                cpu_mask: AtomicUsize::new(0),
            }
        }
    }

    struct BlockingProbe {
        started: crate::task::Completion,
        release: crate::task::Completion,
        completed: AtomicUsize,
    }

    impl BlockingProbe {
        const fn new() -> Self {
            Self {
                started: crate::task::Completion::new(),
                release: crate::task::Completion::new(),
                completed: AtomicUsize::new(0),
            }
        }
    }

    // BlockingProbe stack-size contract: Completion must stay compact. This
    // catches any future regression that embeds O(MAX_TASKS) storage again.
    const _: () = {
        assert!(core::mem::size_of::<BlockingProbe>() <= 1024);
    };

    fn record_callback(argument: usize) {
        // SAFETY: every verifier keeps the Probe alive until flush/cancel_sync
        // has made the callback lifetime quiescent.
        let probe = unsafe { &*(argument as *const Probe) };
        crate::context::assert_task_context();
        crate::context::assert_interrupts_enabled();
        let cpu = crate::smp::current_cpu_id();
        probe
            .cpu_mask
            .fetch_or(1_usize << cpu.get(), Ordering::AcqRel);
        probe.hits.fetch_add(1, Ordering::AcqRel);
    }

    fn sleepable_callback(argument: usize) {
        crate::timer::sleep(Duration::from_millis(2));
        record_callback(argument);
    }

    fn blocking_callback(argument: usize) {
        // SAFETY: the boxed probe remains alive until both work items have
        // completed synchronously.
        let probe = unsafe { &*(argument as *const BlockingProbe) };
        probe.started.complete();
        probe.release.wait();
        probe.completed.fetch_add(1, Ordering::AcqRel);
    }

    #[inline(never)]
    fn verify_immediate_execution() {
        let probe = Probe::new();
        let handle = queue(record_callback, core::ptr::addr_of!(probe) as usize)
            .expect("unable to queue immediate verifier work");
        flush(handle);
        assert_eq!(probe.hits.load(Ordering::Acquire), 1);
    }

    #[inline(never)]
    fn verify_delayed_tickless_execution() {
        let probe = Probe::new();
        let cpu = crate::smp::current_cpu_id();
        let idle_entries = crate::time::tickless_idle_entries_for(cpu);
        let ticks = crate::time::timer_ticks_for(cpu);
        let before = crate::time::now();
        let handle = queue_delayed(
            Duration::from_millis(25),
            record_callback,
            core::ptr::addr_of!(probe) as usize,
        )
        .expect("unable to queue delayed verifier work");
        flush(handle);
        let after = crate::time::now();
        assert!(after.duration_since(before) >= Duration::from_millis(25));
        assert_eq!(probe.hits.load(Ordering::Acquire), 1);
        assert!(
            crate::time::tickless_idle_entries_for(cpu) > idle_entries,
            "delayed work did not pass through tickless idle",
        );
        assert!(
            crate::time::timer_ticks_for(cpu).wrapping_sub(ticks) <= 1,
            "periodic scheduler ticks continued while delayed work was the only deadline",
        );
    }

    #[inline(never)]
    fn verify_synchronous_cancel() {
        let probe = Probe::new();
        let handle = queue_delayed(
            Duration::from_secs(1),
            record_callback,
            core::ptr::addr_of!(probe) as usize,
        )
        .expect("unable to queue cancellation verifier work");
        assert!(cancel_sync(handle));
        crate::timer::sleep(Duration::from_millis(10));
        assert_eq!(probe.hits.load(Ordering::Acquire), 0);
        assert!(is_complete(handle));
    }

    #[inline(never)]
    fn verify_sleepable_callback() {
        let probe = Probe::new();
        let handle = queue(sleepable_callback, core::ptr::addr_of!(probe) as usize)
            .expect("unable to queue sleepable verifier work");
        flush(handle);
        assert_eq!(probe.hits.load(Ordering::Acquire), 1);
    }

    #[inline(never)]
    fn verify_blocked_worker_does_not_stall_pool() {
        let blocking = Box::new(BlockingProbe::new());
        let blocking_pointer = core::ptr::from_ref(blocking.as_ref()) as usize;
        let first = queue(blocking_callback, blocking_pointer)
            .expect("unable to queue blocking verifier work");
        blocking.started.wait();

        let second_probe = Probe::new();
        let second = queue(record_callback, core::ptr::addr_of!(second_probe) as usize)
            .expect("unable to queue concurrent verifier work");
        flush(second);
        assert_eq!(second_probe.hits.load(Ordering::Acquire), 1);
        assert_eq!(blocking.completed.load(Ordering::Acquire), 0);

        blocking.release.complete();
        flush(first);
        assert_eq!(blocking.completed.load(Ordering::Acquire), 1);
    }

    #[inline(never)]
    fn verify_smp_dispatch() {
        let probe = Probe::new();
        let mut handles = Vec::new();
        let mut expected = 0_usize;
        for index in 0..crate::smp::discovered_cpu_count() {
            let cpu = CpuId::new(index).expect("verifier CPU exceeds MAX_CPUS");
            if !crate::smp::is_scheduler_active(cpu) {
                continue;
            }
            expected |= 1_usize << index;
            handles.push(
                queue_on(cpu, record_callback, core::ptr::addr_of!(probe) as usize)
                    .expect("unable to queue SMP verifier work"),
            );
        }
        for handle in handles {
            flush(handle);
        }
        assert_eq!(probe.cpu_mask.load(Ordering::Acquire), expected);
        assert_eq!(
            probe.hits.load(Ordering::Acquire),
            expected.count_ones() as usize
        );
    }

    #[inline(never)]
    fn verify_slot_reclamation() {
        let probe = Probe::new();
        let mut handles = Vec::with_capacity(WORK_SLOTS_PER_CPU);
        for _ in 0..WORK_SLOTS_PER_CPU {
            handles.push(
                queue_delayed(
                    Duration::from_secs(5),
                    record_callback,
                    core::ptr::addr_of!(probe) as usize,
                )
                .expect("workqueue slot reclamation setup failed"),
            );
        }
        assert!(matches!(
            queue_delayed(
                Duration::from_secs(5),
                record_callback,
                core::ptr::addr_of!(probe) as usize,
            ),
            Err(WorkError::Capacity)
        ));
        for handle in handles {
            assert!(cancel_sync(handle));
        }
        assert_eq!(probe.hits.load(Ordering::Acquire), 0);
    }

    pub(super) fn worker() {
        verify_immediate_execution();
        verify_delayed_tickless_execution();
        verify_synchronous_cancel();
        verify_sleepable_callback();
        verify_blocked_worker_does_not_stall_pool();
        verify_smp_dispatch();
        verify_slot_reclamation();

        crate::println!("workqueue runtime test:");
        crate::println!("  immediate work     : verified");
        crate::println!("  delayed work       : verified");
        crate::println!("  synchronous cancel : verified");
        crate::println!("  sleepable callback : verified");
        crate::println!("  worker concurrency : verified");
        crate::println!("  SMP dispatch       : verified");
        crate::println!("  slot reclamation   : verified");
        crate::println!("  tickless wakeup    : verified");
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    crate::task::run_verifier_thread(verify::worker);
}
