// M6-B r4 compact intrusive wait queues.
#[cfg(debug_assertions)]
mod idle_verify;

#[cfg(debug_assertions)]
mod m4c2_verify;
#[cfg(debug_assertions)]
mod m4c_verify;
mod stack;
mod wait_queue;

pub use wait_queue::{Completion, WaitOutcome, WaitQueue};

use alloc::{collections::VecDeque, sync::Arc, vec::Vec};
#[cfg(debug_assertions)]
use core::hint::{black_box, spin_loop};
use core::{
    cell::UnsafeCell,
    marker::PhantomData,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicU8, AtomicUsize, Ordering},
};

use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
    smp::CpuId,
    tracked_spin::{TrackedSpinLock, TrackedSpinLockGuard},
};
use stack::KernelStack;

const MAX_TASKS: usize = 1024;
const DEFAULT_TIME_SLICE_TICKS: u32 = 4;
const MAX_CPUS: usize = crate::smp::MAX_CPUS;
#[cfg(debug_assertions)]
const SINGLE_CPU_VERIFY_ITERATIONS: usize = 50_000;
#[cfg(debug_assertions)]
const SMP_VERIFY_ITERATIONS: usize = 512;
#[cfg(debug_assertions)]
const COOPERATIVE_VERIFY_ITERATIONS: usize = 50_256;
#[cfg(debug_assertions)]
const COOPERATIVE_MINIMUM_SWITCHES: u64 = 100_000;
#[cfg(debug_assertions)]
const STEAL_TASK_COUNT: usize = 16;
#[cfg(debug_assertions)]
const VERIFY_TIMEOUT_SECONDS: u64 = 30;

type ContextSwitch = (
    *mut crate::arch::task::Context,
    *const crate::arch::task::Context,
);

pub type KernelThreadEntry = fn();

#[must_use = "dropping the guard re-enables preemption"]
pub struct PreemptGuard {
    task: TaskId,
    _not_send: PhantomData<*mut ()>,
}

impl PreemptGuard {
    pub fn new() -> Self {
        let task = preempt_disable();
        Self {
            task,
            _not_send: PhantomData,
        }
    }
}

impl Default for PreemptGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PreemptGuard {
    fn drop(&mut self) {
        preempt_enable_task(self.task);
    }
}

#[must_use = "dropping the guard re-enables migration"]
pub struct MigrationGuard {
    task: TaskId,
    _not_send: PhantomData<*mut ()>,
}

impl MigrationGuard {
    /// Independent of PreemptGuard: the holder stays preemptible and may be
    /// scheduled out, but every scheduler placement path (steal, wake,
    /// enqueue) keeps the task on the CPU recorded at the 0->1 depth edge.
    pub fn new() -> Self {
        let task = migration_disable_local();
        Self {
            task,
            _not_send: PhantomData,
        }
    }
}

impl Default for MigrationGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MigrationGuard {
    fn drop(&mut self) {
        migration_enable_task(self.task);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskId(usize);

impl TaskId {
    pub(crate) const fn raw(self) -> usize {
        self.0
    }

    pub(crate) const fn from_raw(raw: usize) -> Self {
        Self(raw)
    }

    fn expect_current(self) -> Self {
        assert_ne!(self.0, NO_CPU, "online CPU has no current task");
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskState {
    Runnable,
    Running(CpuId),
    SwitchingOut(CpuId),
    Blocked,
    Idle(CpuId),
    Exited,
}

impl TaskState {
    // smp-pcpu-v1: task scheduling fields live in atomics so that cross-owner
    // reads (work stealing under a donor rq lock, registry-held scans) are
    // sound without taking the global scheduler lock. The CPU id fits in the
    // high bits because MAX_CPUS <= 8.
    const fn encode(self) -> u8 {
        match self {
            Self::Runnable => 0,
            Self::Running(cpu) => 1 | (cpu.get() as u8) << 3,
            Self::SwitchingOut(cpu) => 2 | (cpu.get() as u8) << 3,
            Self::Blocked => 3,
            Self::Idle(cpu) => 4 | (cpu.get() as u8) << 3,
            Self::Exited => 5,
        }
    }

    fn decode(raw: u8) -> Self {
        let cpu = || CpuId::new((raw >> 3) as usize).expect("invalid CPU id in task state");
        match raw & 0b111 {
            0 => Self::Runnable,
            1 => Self::Running(cpu()),
            2 => Self::SwitchingOut(cpu()),
            3 => Self::Blocked,
            4 => Self::Idle(cpu()),
            5 => Self::Exited,
            _ => unreachable!("invalid encoded task state"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskKind {
    Idle(CpuId),
    KernelThread,
    SystemThread,
    UserThread,
}

impl TaskKind {
    const fn is_idle(self) -> bool {
        matches!(self, Self::Idle(_))
    }

    const fn is_counted_kernel_thread(self) -> bool {
        matches!(self, Self::KernelThread)
    }
}

/// Encoded `Option<CpuId>` for atomic scheduling fields.
const NO_CPU: usize = usize::MAX;

const fn encode_cpu(cpu: Option<CpuId>) -> usize {
    match cpu {
        Some(cpu) => cpu.get(),
        None => NO_CPU,
    }
}

fn decode_cpu(raw: usize) -> Option<CpuId> {
    (raw != NO_CPU).then(|| CpuId::new(raw).expect("invalid CPU id in atomic task field"))
}

struct Task {
    id: TaskId,
    kind: TaskKind,
    // Scheduling fields shared across owners (rq locks, the per-CPU owner,
    // the registry).  Atomics keep cross-owner reads sound; see design.md §3.
    state: AtomicU8,
    queued_on: AtomicUsize,
    affinity: AtomicUsize,
    has_run: AtomicBool,
    // Everything below is registry-owned: mutated only while holding the
    // REGISTRY lock, except `context` which the architecture writes through
    // the UnsafeCell while the task is SwitchingOut/Running and no other
    // CPU can reference it.
    //
    // Preemption and migration-disable depths are Task-owned single
    // authorities: the running task bumps them locklessly on its own CPU
    // and every scheduler gate reads them from the task record, so no
    // CPU-local mirror can drift out of sync across a switch.
    preempt_disable_depth: AtomicUsize,
    migration_disable_depth: AtomicUsize,
    /// Tracked-spin-lock hold count, Task-owned. A force-exit must not
    /// abandon the interrupted context while this is non-zero: the guard
    /// would leak and every waiter would spin forever (§10 情况A).
    tracked_lock_depth: AtomicUsize,
    /// CPU the task must not leave while `migration_disable_depth > 0`;
    /// `NO_CPU` when unpinned. Written on the depth 0->1 / 1->0 edges.
    migration_pinned: AtomicUsize,
    context: UnsafeCell<crate::arch::task::Context>,
    stack: Option<KernelStack>,
    entry: Option<KernelThreadEntry>,
    user_thread: Option<Arc<crate::process::Thread>>,
    exit_visible: Option<Arc<Completion>>,
    process_cleanup: Option<ProcessCleanup>,
    wait_channel: Option<usize>,
    wait_queue_address: Option<usize>,
    wait_prev: Option<TaskId>,
    wait_next: Option<TaskId>,
    wake_after_switch: bool,
}

impl Task {
    fn state(&self) -> TaskState {
        TaskState::decode(self.state.load(Ordering::Acquire))
    }

    fn store_state(&self, state: TaskState) {
        self.state.store(state.encode(), Ordering::Release);
    }

    fn queued_on(&self) -> Option<CpuId> {
        decode_cpu(self.queued_on.load(Ordering::Acquire))
    }

    fn store_queued_on(&self, cpu: Option<CpuId>) {
        self.queued_on.store(encode_cpu(cpu), Ordering::Release);
    }

    fn affinity(&self) -> Option<CpuId> {
        decode_cpu(self.affinity.load(Ordering::Acquire))
    }

    fn store_affinity(&self, cpu: Option<CpuId>) {
        self.affinity.store(encode_cpu(cpu), Ordering::Release);
    }

    /// CPU recorded by the 0->1 migration-depth edge; `None` while unpinned.
    fn pinned_cpu(&self) -> Option<CpuId> {
        decode_cpu(self.migration_pinned.load(Ordering::Acquire))
    }

    fn context_ptr(&self) -> *mut crate::arch::task::Context {
        self.context.get()
    }

    fn context(&self) -> &crate::arch::task::Context {
        // Read-only diagnostics; the architecture only writes through
        // context_ptr() while the task is switching.
        unsafe { &*self.context.get() }
    }
}

fn fresh_task_context(
    stack: &KernelStack,
    entry: unsafe extern "C" fn() -> !,
) -> crate::arch::task::Context {
    // One global constructor serves idle tasks, counted kthreads, and permanent
    // system threads.  The saved SP is fully valid before run-queue publication.
    let initial_sp = stack.initial_stack_pointer();
    let context = crate::arch::task::Context::new(initial_sp, entry);
    let saved_sp = context.saved_stack_pointer();

    assert_eq!(
        saved_sp, initial_sp,
        "architecture context changed the validated fresh-task SP",
    );
    assert!(
        stack.contains(saved_sp),
        "fresh task context published an unmapped kernel SP",
    );
    assert_eq!(
        stack.upper_headroom(saved_sp),
        crate::arch::task::FRESH_TASK_STACK_RESERVE,
        "fresh task context lost its architecture bootstrap reserve",
    );
    context
}

impl Task {
    fn boot() -> Self {
        Self {
            id: TaskId(0),
            kind: TaskKind::Idle(CpuId::BOOT),
            state: AtomicU8::new(TaskState::Running(CpuId::BOOT).encode()),
            context: UnsafeCell::new(crate::arch::task::Context::default()),
            stack: None,
            entry: None,
            user_thread: None,
            exit_visible: None,
            process_cleanup: None,
            affinity: AtomicUsize::new(encode_cpu(Some(CpuId::BOOT))),
            queued_on: AtomicUsize::new(NO_CPU),
            has_run: AtomicBool::new(true),
            preempt_disable_depth: AtomicUsize::new(0),
            migration_disable_depth: AtomicUsize::new(0),
            tracked_lock_depth: AtomicUsize::new(0),
            migration_pinned: AtomicUsize::new(NO_CPU),
            wait_channel: None,
            wait_queue_address: None,
            wait_prev: None,
            wait_next: None,
            wake_after_switch: false,
        }
    }

    fn idle(id: TaskId, cpu: CpuId, stack: KernelStack) -> Self {
        Self {
            id,
            kind: TaskKind::Idle(cpu),
            state: AtomicU8::new(TaskState::Idle(cpu).encode()),
            context: UnsafeCell::new(fresh_task_context(&stack, idle_thread_bootstrap)),
            stack: Some(stack),
            entry: None,
            user_thread: None,
            exit_visible: None,
            process_cleanup: None,
            affinity: AtomicUsize::new(encode_cpu(Some(cpu))),
            queued_on: AtomicUsize::new(NO_CPU),
            has_run: AtomicBool::new(false),
            preempt_disable_depth: AtomicUsize::new(0),
            migration_disable_depth: AtomicUsize::new(0),
            tracked_lock_depth: AtomicUsize::new(0),
            migration_pinned: AtomicUsize::new(NO_CPU),
            wait_channel: None,
            wait_queue_address: None,
            wait_prev: None,
            wait_next: None,
            wake_after_switch: false,
        }
    }

    fn kernel_thread(
        id: TaskId,
        entry: KernelThreadEntry,
        stack: KernelStack,
        affinity: Option<CpuId>,
        kind: TaskKind,
    ) -> Self {
        assert!(
            matches!(kind, TaskKind::KernelThread | TaskKind::SystemThread),
            "invalid kernel thread kind",
        );
        Self {
            id,
            kind,
            state: AtomicU8::new(TaskState::Runnable.encode()),
            context: UnsafeCell::new(fresh_task_context(&stack, kernel_thread_bootstrap)),
            stack: Some(stack),
            entry: Some(entry),
            user_thread: None,
            exit_visible: None,
            process_cleanup: None,
            affinity: AtomicUsize::new(encode_cpu(affinity)),
            queued_on: AtomicUsize::new(NO_CPU),
            has_run: AtomicBool::new(false),
            preempt_disable_depth: AtomicUsize::new(0),
            migration_disable_depth: AtomicUsize::new(0),
            tracked_lock_depth: AtomicUsize::new(0),
            migration_pinned: AtomicUsize::new(NO_CPU),
            wait_channel: None,
            wait_queue_address: None,
            wait_prev: None,
            wait_next: None,
            wake_after_switch: false,
        }
    }

    fn user_thread(
        id: TaskId,
        thread: Arc<crate::process::Thread>,
        exit_visible: Arc<Completion>,
        process_cleanup: Option<ProcessCleanup>,
        stack: KernelStack,
        affinity: Option<CpuId>,
    ) -> Self {
        thread
            .bind_scheduler_task(id)
            .expect("M9-B failed to bind Thread to scheduler task");
        Self {
            id,
            kind: TaskKind::UserThread,
            state: AtomicU8::new(TaskState::Runnable.encode()),
            context: UnsafeCell::new(fresh_task_context(&stack, user_thread_bootstrap)),
            stack: Some(stack),
            entry: None,
            user_thread: Some(thread),
            exit_visible: Some(exit_visible),
            process_cleanup,
            affinity: AtomicUsize::new(encode_cpu(affinity)),
            queued_on: AtomicUsize::new(NO_CPU),
            has_run: AtomicBool::new(false),
            preempt_disable_depth: AtomicUsize::new(0),
            migration_disable_depth: AtomicUsize::new(0),
            tracked_lock_depth: AtomicUsize::new(0),
            migration_pinned: AtomicUsize::new(NO_CPU),
            wait_channel: None,
            wait_queue_address: None,
            wait_prev: None,
            wait_next: None,
            wake_after_switch: false,
        }
    }

    fn user_mm(&self) -> Option<Arc<crate::user_mm::UserMm>> {
        self.user_thread
            .as_ref()
            .map(|thread| thread.process().mm_arc())
    }

    fn stack_contains(&self, address: usize) -> bool {
        self.stack
            .as_ref()
            .is_some_and(|stack| stack.contains(address))
    }

    fn destroy_resources(mut self) {
        assert!(
            self.queued_on().is_none(),
            "destroying task still linked to a run queue: {:?}",
            self.id,
        );
        assert!(
            self.wait_channel.is_none(),
            "destroying task still linked to wait channel {:?}: {:?}",
            self.wait_channel,
            self.id,
        );
        assert!(
            self.wait_queue_address.is_none(),
            "destroying task retained a wait-queue address: {:?}",
            self.id,
        );
        assert!(
            !self.wake_after_switch,
            "destroying task with a pending wake claim: {:?}",
            self.id,
        );
        assert!(
            self.wait_prev.is_none() && self.wait_next.is_none(),
            "destroying task retained intrusive wait links: {:?}",
            self.id,
        );
        if let Some(stack) = self.stack.take() {
            stack
                .recycle()
                .unwrap_or_else(|error| panic!("unable to release kernel stack: {error:?}"));
        }
        assert!(
            self.exit_visible.is_none(),
            "exit-visible completion reached final task reclamation",
        );
        if let Some(cleanup) = self.process_cleanup.take() {
            // ExitVisible and final reclamation are independent.  The reaper
            // must not wait for a synchronous caller, otherwise one delayed or
            // aborted caller stalls reclamation of every later retired task.
            //
            // Preserve stack -> Thread -> Process drop order. Process teardown
            // is RAII-backed, so legitimate temporary Arc<Process> readers do
            // not turn reclamation into a kernel panic.
            drop(self.user_thread.take());
            drop(cleanup.process);
        } else {
            drop(self.user_thread.take());
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SwitchDisposition {
    Yield,
    Block,
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingSwitch {
    previous: TaskId,
    next: TaskId,
    disposition: SwitchDisposition,
}

struct ExitVisible {
    completion: Arc<Completion>,
}

struct ProcessCleanup {
    process: Arc<crate::process::Process>,
}

struct CompletedSwitch {
    retired_task_added: bool,
    exit_visible: Option<ExitVisible>,
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct WaiterDebugState {
    pub blocked: usize,
    pub switching: usize,
    pub claimed_switching: usize,
}

// smp-pcpu-v1: per-CPU scheduler storage.  Truly-local fields (irq_depth,
// timeslice, pending, stats) are written only by the owning CPU;
// `current`/`idle`/`need_resched` are atomics so cross-CPU diagnostics and
// remote wakeups stay sound.  Preemption and migration depth live on the
// Task, not here, so they survive the switch with their owner.  The
// runqueue owns Runnable tasks.
struct RunQueue {
    lock: TrackedSpinLock<VecDeque<TaskId>>,
}

impl RunQueue {
    const fn new() -> Self {
        Self {
            lock: TrackedSpinLock::new_with_class(
                VecDeque::new(),
                LockClass::new("runqueue", LockRank::RunQueue, 1),
            ),
        }
    }

    fn lock(&self) -> TrackedSpinLockGuard<'_, VecDeque<TaskId>> {
        self.lock.lock()
    }

    fn try_lock(&self) -> Option<TrackedSpinLockGuard<'_, VecDeque<TaskId>>> {
        self.lock.try_lock()
    }
}

// PendingSwitch is packed into one AtomicU64 so cross-CPU diagnostics
// (debug dumps, stack-owner repair) read it without racing the owning CPU.
const PENDING_RAW_NONE: u64 = 0;
const PENDING_SHIFT: u32 = 11; // MAX_TASKS = 1024 fits in 11 bits
const PENDING_DISPOSITION_SHIFT: u32 = 22;

fn encode_pending(pending: PendingSwitch) -> u64 {
    let disposition = match pending.disposition {
        SwitchDisposition::Yield => 0_u64,
        SwitchDisposition::Block => 1_u64,
        SwitchDisposition::Exit => 2_u64,
    };
    // No presence bit: a real pending {0, 0} is unreachable (every prepare
    // asserts previous != next and the idle task never blocks or exits), so
    // raw == 0 already means "no transaction".  OR-ing a 1 here would alias
    // the low bit of `previous` and corrupt every decode.
    (pending.previous.raw() as u64)
        | (pending.next.raw() as u64) << PENDING_SHIFT
        | disposition << PENDING_DISPOSITION_SHIFT
}

fn decode_pending(raw: u64) -> Option<PendingSwitch> {
    if raw == PENDING_RAW_NONE {
        return None;
    }
    let disposition = match (raw >> PENDING_DISPOSITION_SHIFT) & 0b11 {
        0 => SwitchDisposition::Yield,
        1 => SwitchDisposition::Block,
        2 => SwitchDisposition::Exit,
        _ => unreachable!("invalid encoded switch disposition"),
    };
    Some(PendingSwitch {
        previous: TaskId::from_raw((raw as usize) & ((1_usize << PENDING_SHIFT) - 1)),
        next: TaskId::from_raw(((raw >> PENDING_SHIFT) as usize) & ((1_usize << PENDING_SHIFT) - 1)),
        disposition,
    })
}

struct CpuLocal {
    current: AtomicUsize, // encoded TaskId; NO_CPU when offline
    idle: AtomicUsize,
    rq: RunQueue,
    need_resched: AtomicBool,
    timeslice_remaining: u32,
    irq_depth: usize,
    pending: AtomicU64, // encoded PendingSwitch; 0 = none
    loaded_mm: UnsafeCell<Option<Arc<crate::user_mm::UserMm>>>,
    context_switches: AtomicU64,
    preemptions: AtomicU64,
    mm_switches: AtomicU64,
    // §10 TLB-shootdown timeout diagnostics: owned-CPU writes, remote reads
    // only from the panic path where a racy snapshot is sound by design.
    irq_mirror: AtomicU8,     // 1 = last known local interrupt state enabled
    irq_mirror_at: AtomicU64, // time::counter() at the last mirror update
    in_trap_depth: AtomicUsize, // > 0 while inside kernel_arch_trap (SIE = 0)
    last_switch: AtomicU64,   // time::counter() at the last committed switch
    // §10: trap continuations that resumed on a different CPU than the one
    // that entered them (switch-inside-trap migration) leave the per-CPU
    // depth unbalanced; the exit side saturates and counts those events.
    trap_imbalance: AtomicUsize,
}

impl CpuLocal {
    const fn new() -> Self {
        Self {
            current: AtomicUsize::new(NO_CPU),
            idle: AtomicUsize::new(NO_CPU),
            rq: RunQueue::new(),
            need_resched: AtomicBool::new(false),
            timeslice_remaining: DEFAULT_TIME_SLICE_TICKS,
            irq_depth: 0,
            pending: AtomicU64::new(PENDING_RAW_NONE),
            loaded_mm: UnsafeCell::new(None),
            context_switches: AtomicU64::new(0),
            preemptions: AtomicU64::new(0),
            mm_switches: AtomicU64::new(0),
            irq_mirror: AtomicU8::new(0),
            irq_mirror_at: AtomicU64::new(0),
            in_trap_depth: AtomicUsize::new(0),
            last_switch: AtomicU64::new(0),
            trap_imbalance: AtomicUsize::new(0),
        }
    }

    fn pending(&self) -> Option<PendingSwitch> {
        decode_pending(self.pending.load(Ordering::Acquire))
    }

    fn store_pending(&self, pending: Option<PendingSwitch>) {
        self.pending.store(
            pending.map_or(PENDING_RAW_NONE, encode_pending),
            Ordering::Release,
        );
    }

    /// Loaded-MM accessors use UnsafeCell interior mutability: the owning
    /// CPU reads/writes it in the scheduling path while remote CPUs may only
    /// read it from quiescent debug verifiers (after all user tasks exited
    /// and every switch committed).
    fn loaded_mm(&self) -> Option<Arc<crate::user_mm::UserMm>> {
        // SAFETY: see CpuLocal::loaded_mm field contract.
        unsafe { (*self.loaded_mm.get()).clone() }
    }

    fn store_loaded_mm(&self, mm: Option<Arc<crate::user_mm::UserMm>>) {
        // SAFETY: see CpuLocal::loaded_mm field contract.
        unsafe {
            *self.loaded_mm.get() = mm;
        }
    }

    fn take_loaded_mm(&self) -> Option<Arc<crate::user_mm::UserMm>> {
        // SAFETY: see CpuLocal::loaded_mm field contract.
        unsafe { (*self.loaded_mm.get()).take() }
    }
}

struct CpuLocalSlot(UnsafeCell<CpuLocal>);

// SAFETY: truly-local fields are mutated only by the owning CPU; cross-CPU
// accesses use the atomics or the documented accessors on CpuLocal.
unsafe impl Sync for CpuLocalSlot {}

static CPUS: [CpuLocalSlot; MAX_CPUS] =
    [const { CpuLocalSlot(UnsafeCell::new(CpuLocal::new())) }; MAX_CPUS];

fn cpu_local(cpu: CpuId) -> &'static CpuLocal {
    // SAFETY: CPUS is write-once at initialization; readers may be remote
    // CPUs for atomic/diagnostic fields only.
    unsafe { &*CPUS[cpu.get()].0.get() }
}

fn cpu_local_mut(cpu: CpuId) -> &'static mut CpuLocal {
    debug_assert_eq!(
        crate::smp::current_cpu_id(),
        cpu,
        "cross-CPU CpuLocal mutation"
    );
    // SAFETY: truly-local fields are only mutated by the owning CPU.
    unsafe { &mut *CPUS[cpu.get()].0.get() }
}

/// Stable base pointer to the registry task table.  The Vec is created with
/// capacity MAX_TASKS and its length never exceeds MAX_TASKS, so it never
/// reallocates; a Runnable task's slot is written exactly once (at spawn,
/// before rq publication) and only taken when the task exits on its own CPU.
/// This lets the local fast path obtain `&Task` without the REGISTRY lock.
static TASKS_BASE: AtomicPtr<Option<Task>> = AtomicPtr::new(core::ptr::null_mut());

fn task_ref(id: TaskId) -> &'static Task {
    let base = TASKS_BASE.load(Ordering::Acquire);
    assert!(!base.is_null(), "task table not initialized");
    // SAFETY: the slot is stable (see TASKS_BASE comment) and the caller's
    // ownership (rq lock / own CPU) guarantees it is not vacant.
    unsafe { (&*base.add(id.0)).as_ref().expect("task slot is vacant") }
}

/// Number of discovered CPUs, mirrored for lockless fast paths.
static DISCOVERED_CPUS: AtomicUsize = AtomicUsize::new(0);

/// Lockless initialization gate for the per-CPU fast paths.  REGISTRY-heavy
/// paths may still take SCHEDULER directly.
fn scheduler_ready() -> bool {
    !TASKS_BASE.load(Ordering::Acquire).is_null()
}

fn current_of(cpu: CpuId) -> TaskId {
    assert!(
        scheduler_ready(),
        "kernel scheduler is not initialized"
    );
    let raw = cpu_local(cpu).current.load(Ordering::Acquire);
    TaskId::from_raw(raw).expect_current()
}

fn idle_of(cpu: CpuId) -> TaskId {
    assert!(
        scheduler_ready(),
        "kernel scheduler is not initialized"
    );
    let raw = cpu_local(cpu).idle.load(Ordering::Acquire);
    TaskId::from_raw(raw).expect_current()
}

// smp-pcpu-v1: rq manipulation runs under the per-CPU runqueue lock only.
// All callers hold local interrupts disabled (IrqSaveGuard, REGISTRY's
// IrqSpinLock, or trap context), so a timer hardirq can never preempt an
// rq critical section.

fn enqueue_task(id: TaskId, cpu: CpuId) {
    let task = task_ref(id);
    assert_eq!(task.state(), TaskState::Runnable);
    assert!(task.queued_on().is_none(), "task was queued more than once");
    if let Some(affinity) = task.affinity() {
        assert_eq!(affinity, cpu, "pinned task queued on the wrong CPU");
    }
    if let Some(pinned) = task.pinned_cpu() {
        assert_eq!(pinned, cpu, "migration-pinned task queued on the wrong CPU");
    }
    task.store_queued_on(Some(cpu));
    cpu_local(cpu).rq.lock().push_back(id);
}

fn dequeue_local_task(cpu: CpuId) -> Option<TaskId> {
    let id = cpu_local(cpu).rq.lock().pop_front()?;
    let task = task_ref(id);
    assert_eq!(task.state(), TaskState::Runnable);
    assert_eq!(task.queued_on(), Some(cpu));
    task.store_queued_on(None);
    Some(id)
}

fn steal_runnable_task(cpu: CpuId) -> Option<TaskId> {
    let discovered = DISCOVERED_CPUS.load(Ordering::Acquire);
    for donor_index in 0..discovered {
        let donor = CpuId::new(donor_index).expect("invalid donor CPU");
        if donor == cpu || !crate::smp::is_scheduler_active(donor) {
            continue;
        }

        let mut queue = cpu_local(donor).rq.lock();
        let position = queue.iter().position(|id| {
            let task = task_ref(*id);
            // Linux-style work stealing: a runnable task without explicit
            // affinity may move after any completed switch-out. The
            // architecture context carries FP/vector state, and the user
            // trap anchor is rebuilt from the destination CPU before
            // returning to user mode.
            task.state() == TaskState::Runnable
                && task.affinity().is_none()
                && task.migration_disable_depth.load(Ordering::Acquire) == 0
        });
        let Some(position) = position else {
            continue;
        };
        let id = queue
            .remove(position)
            .expect("stealable task disappeared from donor queue");
        drop(queue);

        let task = task_ref(id);
        assert_eq!(task.queued_on(), Some(donor));
        task.store_queued_on(None);
        return Some(id);
    }

    None
}

fn dequeue_next_task(cpu: CpuId) -> Option<TaskId> {
    dequeue_local_task(cpu).or_else(|| steal_runnable_task(cpu))
}

fn activate_next_task(id: TaskId, cpu: CpuId) {
    let task = task_ref(id);

    match task.kind {
        TaskKind::Idle(owner) => {
            assert_eq!(owner, cpu, "idle task selected by the wrong CPU");
            assert_eq!(task.state(), TaskState::Idle(cpu));
        }
        TaskKind::KernelThread | TaskKind::SystemThread => {
            assert_eq!(task.state(), TaskState::Runnable);
            if let Some(affinity) = task.affinity() {
                assert_eq!(affinity, cpu, "pinned task selected by the wrong CPU");
            }
            task.has_run.store(true, Ordering::Release);
        }
        TaskKind::UserThread => {
            assert_eq!(task.state(), TaskState::Runnable);
            if let Some(affinity) = task.affinity() {
                assert_eq!(affinity, cpu, "pinned user task selected by the wrong CPU",);
            }
            task.has_run.store(true, Ordering::Release);
        }
    }

    assert!(task.queued_on().is_none());
    task.store_state(TaskState::Running(cpu));

    let local = cpu_local_mut(cpu);
    local.timeslice_remaining = DEFAULT_TIME_SLICE_TICKS;
    local.need_resched.store(false, Ordering::Release);
}

fn context_pair(previous: TaskId, next: TaskId) -> ContextSwitch {
    (task_ref(previous).context_ptr(), task_ref(next).context_ptr())
}

fn cpu_runnable_load(cpu: CpuId) -> usize {
    let queued = cpu_local(cpu).rq.lock().len();
    let running = (cpu_local(cpu).current.load(Ordering::Acquire) != NO_CPU)
        .then(|| usize::from(!task_ref(current_of(cpu)).kind.is_idle()))
        .unwrap_or(0);
    queued.saturating_add(running)
}

/// Install the incoming task's user address space on this CPU.  The
/// previous/next user_mm reads and the loaded-mm bookkeeping are all
/// CPU-local: the call happens inside the switch transaction while local
/// interrupts are disabled, before `pending` is published.
fn switch_mm_irqs_off(cpu: CpuId, previous: TaskId, next: TaskId) {
    assert!(
        crate::arch::interrupt::are_disabled(),
        "M9-B switch_mm_irqs_off ran with local interrupts enabled",
    );
    debug_assert_eq!(
        crate::smp::current_cpu_id(),
        cpu,
        "M9-B switch_mm_irqs_off ran off-CPU"
    );

    let previous_mm = task_ref(previous).user_mm();
    let next_mm = task_ref(next).user_mm();
    let local = cpu_local(cpu);
    let mut loaded_mm = local.loaded_mm();

    let mismatch = match (&previous_mm, &loaded_mm) {
        (None, None) => false,
        (Some(previous), Some(loaded)) => !Arc::ptr_eq(previous, loaded),
        (Some(_), None) | (None, Some(_)) => true,
    };

    if mismatch {
        #[cfg(debug_assertions)]
        crate::println!(
            "scheduler: loaded-mm mismatch cpu={} prev_user={} loaded={}; repairing",
            cpu.get(),
            previous_mm.is_some(),
            loaded_mm.is_some(),
        );
        // Deactivate stale loaded mm if present.
        if let Some(stale) = &loaded_mm {
            stale
                .deactivate_current_cpu()
                .unwrap_or_else(|error| panic!("M9-B failed to repair stale mm: {error:?}"));
        }
        local.store_loaded_mm(None);
        loaded_mm = None;
        // Fall through: the activation path below will set up the
        // correct mm for the incoming task.
    }

    if let (Some(loaded), Some(incoming)) = (&loaded_mm, &next_mm)
        && Arc::ptr_eq(loaded, incoming)
    {
        if let Some(thread) = task_ref(next).user_thread.as_ref() {
            thread.record_cpu(cpu);
        }
        return;
    }

    local
        .mm_switches
        .fetch_add(1, Ordering::Relaxed);

    if let Some(loaded) = loaded_mm {
        loaded
            .deactivate_current_cpu()
            .unwrap_or_else(|error| panic!("M9-B failed to leave outgoing mm: {error:?}"));
        local.store_loaded_mm(None);
    }

    if let Some(next_mm) = next_mm {
        next_mm
            .activate_current_cpu()
            .unwrap_or_else(|error| panic!("M9-B failed to enter incoming mm: {error:?}"));
        local.store_loaded_mm(Some(next_mm));
        if let Some(thread) = task_ref(next).user_thread.as_ref() {
            thread.record_cpu(cpu);
        }
    }
}

fn prepare_yield_local(cpu: CpuId) -> Option<ContextSwitch> {
    assert!(
        crate::smp::is_scheduler_active(cpu),
        "inactive CPU attempted to schedule"
    );
    assert!(
        cpu_local(cpu).pending().is_none(),
        "CPU attempted a nested context switch",
    );

    if task_ref(current_of(cpu))
        .preempt_disable_depth
        .load(Ordering::Acquire)
        != 0
    {
        // The running task holds preemption disabled: switching it out would
        // leak the pin onto the next task. Yield is only a hint; stay local.
        return None;
    }

    #[cfg(debug_assertions)]
    if crate::tracked_spin::preemptible_lock_depth(cpu) != 0 {
        // Yield is a hint: refusing to switch a preemptible-lock holder keeps
        // the per-CPU lockdep stack LIFO-consistent (see prepare_preempt_local).
        return None;
    }

    let previous = current_of(cpu);
    assert_eq!(task_ref(previous).state(), TaskState::Running(cpu));

    let Some(next) = dequeue_next_task(cpu) else {
        // A runnable current task must never yield to the idle task.
        // Linux keeps the sole runnable task selected; idle is only a
        // fallback for block/exit or when the current task is already idle.
        //
        // Switching a runnable task to idle and re-enqueuing it in
        // switch-tail creates a runnable-without-wakeup window under
        // NO_HZ. Treat yield as a hint and continue locally when there is
        // no alternative task.
        let local = cpu_local_mut(cpu);
        local.need_resched.store(false, Ordering::Release);
        local.timeslice_remaining = DEFAULT_TIME_SLICE_TICKS;
        return None;
    };

    assert_ne!(previous, next, "CPU selected its current task as next");
    task_ref(previous).store_state(TaskState::SwitchingOut(cpu));
    activate_next_task(next, cpu);
    switch_mm_irqs_off(cpu, previous, next);
    {
        let local = cpu_local_mut(cpu);
        local.need_resched.store(false, Ordering::Release);
        local.store_pending(Some(PendingSwitch {
            previous,
            next,
            disposition: SwitchDisposition::Yield,
        }));
        local.context_switches.fetch_add(1, Ordering::Relaxed);
        local
            .last_switch
            .store(crate::arch::time::counter(), Ordering::Relaxed);
    }

    Some(context_pair(previous, next))
}

fn prepare_preempt_local(cpu: CpuId) -> Option<ContextSwitch> {
    assert!(
        crate::smp::is_scheduler_active(cpu),
        "inactive CPU attempted to preempt",
    );
    {
        let local = cpu_local(cpu);
        if local.irq_depth != 0 {
            return None;
        }
        // An interrupt can be delivered after the hardware stack changed but
        // before switch-tail commits `pending.next` as current. The existing
        // switch owns this CPU; defer the tick instead of nesting a second
        // context switch into the same per-CPU transaction.
        if local.pending().is_some() {
            return None;
        }
        if task_ref(current_of(cpu))
            .preempt_disable_depth
            .load(Ordering::Acquire)
            != 0
        {
            return None;
        }
        if !local.need_resched.load(Ordering::Acquire) {
            return None;
        }
    }

    #[cfg(debug_assertions)]
    if crate::tracked_spin::preemptible_lock_depth(cpu) != 0 {
        // The per-CPU LIFO lockdep stack requires that a tracked-lock holder
        // is never switched away mid-critical-section: a later task would push
        // its own entries above the suspended holder's, and the holder's
        // release would then pop the wrong instance. Preemptible locks carry
        // no migration guard, so the preempt gate enforces it here instead.
        // need_resched stays armed and the tick is retried at the next
        // IRQ exit, once the lock is released.
        return None;
    }

    let previous = current_of(cpu);
    assert_eq!(task_ref(previous).state(), TaskState::Running(cpu));

    let Some(next) = dequeue_next_task(cpu) else {
        let local = cpu_local_mut(cpu);
        local.need_resched.store(false, Ordering::Release);
        local.timeslice_remaining = DEFAULT_TIME_SLICE_TICKS;
        return None;
    };

    assert_ne!(previous, next, "CPU selected its current task as next");
    task_ref(previous).store_state(TaskState::SwitchingOut(cpu));
    activate_next_task(next, cpu);
    switch_mm_irqs_off(cpu, previous, next);
    {
        let local = cpu_local_mut(cpu);
        local.need_resched.store(false, Ordering::Release);
        local.store_pending(Some(PendingSwitch {
            previous,
            next,
            disposition: SwitchDisposition::Yield,
        }));
        local.context_switches.fetch_add(1, Ordering::Relaxed);
        local
            .last_switch
            .store(crate::arch::time::counter(), Ordering::Relaxed);
        local.preemptions.fetch_add(1, Ordering::Relaxed);
    }

    Some(context_pair(previous, next))
}

/// Switch-tail commit for the lockless Yield disposition: only the local rq
/// and task atomics are touched, no REGISTRY.
fn complete_yield_switch(cpu: CpuId, pending: PendingSwitch, running_sp: usize) -> bool {
    assert_eq!(
        current_of(cpu),
        pending.previous,
        "scheduler current changed before the hardware stack switch committed",
    );
    assert_eq!(
        task_ref(pending.next).state(),
        TaskState::Running(cpu),
        "incoming task was not Running before switch commit",
    );
    #[cfg(debug_assertions)]
    assert!(
        task_ref(pending.next).stack.is_none()
            || task_ref(pending.next).stack_contains(running_sp),
        "incoming hardware SP is outside the selected task stack: task={:?} sp={running_sp:#x}",
        pending.next,
    );
    cpu_local(cpu)
        .current
        .store(pending.next.raw(), Ordering::Release);
    assert_eq!(
        task_ref(pending.previous).state(),
        TaskState::SwitchingOut(cpu),
    );

    if task_ref(pending.previous).kind.is_idle() {
        task_ref(pending.previous).store_state(TaskState::Idle(cpu));
    } else {
        task_ref(pending.previous).store_state(TaskState::Runnable);
        enqueue_task(pending.previous, cpu);
    }
    task_ref(pending.next).kind.is_idle()
}

fn assert_schedulable(cpu: CpuId) {
    let local = cpu_local(cpu);
    assert_eq!(
        local.irq_depth, 0,
        "task attempted to schedule in IRQ context"
    );
    assert_eq!(
        task_ref(current_of(cpu))
            .preempt_disable_depth
            .load(Ordering::Acquire),
        0,
        "task attempted to schedule with preemption disabled: cpu={} current={:?} tracked={:?}",
        cpu.get(),
        current_of(cpu),
        crate::tracked_spin::held_diagnostic(cpu)
    );
}

fn timer_ticks_local(cpu: CpuId, ticks: u64) {
    if ticks == 0 {
        return;
    }
    let current = current_of(cpu);
    if task_ref(current).kind.is_idle() {
        return;
    }
    let elapsed = u32::try_from(ticks).unwrap_or(u32::MAX);
    let local = cpu_local_mut(cpu);
    if elapsed < local.timeslice_remaining {
        local.timeslice_remaining -= elapsed;
    } else {
        local.timeslice_remaining = 0;
        local.need_resched.store(true, Ordering::Release);
    }
}

/// Lockless preemption pin. The Task-owned depth is the single authority:
/// the running task bumps it on its own CPU and every scheduler gate reads
/// it from the task record, so the count cannot drift across a switch the
/// way a CpuLocal mirror could.
fn preempt_disable_local() -> TaskId {
    assert!(
        scheduler_ready(),
        "preemption used before scheduler initialization"
    );
    let cpu = crate::smp::current_cpu_id();
    let task = current_of(cpu);
    let running_sp = crate::arch::task::current_stack_pointer();
    assert!(
        task_ref(task).stack.is_none() || task_ref(task).stack_contains(running_sp),
        "preemption pin recorded on a foreign stack: task={task:?} sp={running_sp:#x}",
    );
    task_ref(task)
        .preempt_disable_depth
        .fetch_add(1, Ordering::AcqRel);
    task
}

fn preempt_enable_local(task: TaskId) {
    assert!(
        scheduler_ready(),
        "preemption used before scheduler initialization"
    );
    let cpu = crate::smp::current_cpu_id();
    let current = current_of(cpu);
    assert_eq!(
        current, task,
        "preemption guard task disagrees with scheduler current",
    );
    let previous = task_ref(task)
        .preempt_disable_depth
        .fetch_sub(1, Ordering::AcqRel);
    assert!(previous >= 1, "preempt counter underflowed");
    let should_schedule = previous == 1
        && cpu_local(cpu).irq_depth == 0
        && cpu_local(cpu).need_resched.load(Ordering::Acquire);
    if should_schedule && crate::arch::interrupt::are_enabled() {
        preempt_schedule();
    }
}

/// Lockless migration pin. The Task-owned depth is the single authority;
/// the 0->1 edge records the CPU the task must not leave. Unlike a
/// PreemptGuard this does not disable preemption: the pinned task may be
/// scheduled out and is resumed on the recorded CPU by the steal/wake
/// placement rules.
fn migration_disable_local() -> TaskId {
    assert!(
        scheduler_ready(),
        "migration pin used before scheduler initialization"
    );
    let cpu = crate::smp::current_cpu_id();
    let task = current_of(cpu);
    let previous = task_ref(task)
        .migration_disable_depth
        .fetch_add(1, Ordering::AcqRel);
    if previous == 0 {
        task_ref(task)
            .migration_pinned
            .store(cpu.get(), Ordering::Release);
    }
    task
}

fn migration_enable_task(task: TaskId) {
    assert!(
        scheduler_ready(),
        "migration pin used before scheduler initialization"
    );
    let cpu = crate::smp::current_cpu_id();
    let current = current_of(cpu);
    assert_eq!(
        current, task,
        "migration guard task disagrees with scheduler current",
    );
    let depth = task_ref(task)
        .migration_disable_depth
        .fetch_sub(1, Ordering::AcqRel);
    assert!(depth >= 1, "migration depth underflowed");
    if depth == 1 {
        task_ref(task)
            .migration_pinned
            .store(NO_CPU, Ordering::Release);
    }
}

/// Tracked-spin-lock hold accounting, Task-owned. A preemptible lock may be
/// held across a switch (the holder migrates), so the depth must live on the
/// task, not on a CPU. `handle_forced_exit` consults it before abandoning an
/// interrupted context: abandoning a context that holds a tracked lock leaks
/// the guard and leaves every waiter spinning (§10 情况A).
pub(crate) fn note_tracked_lock_acquire() {
    if !scheduler_ready() {
        return;
    }
    let task = current_of(crate::smp::current_cpu_id());
    task_ref(task)
        .tracked_lock_depth
        .fetch_add(1, Ordering::AcqRel);
}

pub(crate) fn note_tracked_lock_release() {
    if !scheduler_ready() {
        return;
    }
    let task = current_of(crate::smp::current_cpu_id());
    let depth = task_ref(task)
        .tracked_lock_depth
        .fetch_sub(1, Ordering::AcqRel);
    assert!(depth >= 1, "tracked-lock depth underflowed");
}

pub(crate) fn current_tracked_lock_depth() -> usize {
    if !scheduler_ready() {
        return 0;
    }
    let task = current_of(crate::smp::current_cpu_id());
    task_ref(task).tracked_lock_depth.load(Ordering::Acquire)
}

fn work_available(cpu: CpuId) -> bool {
    // rq locks carry a rank above Timer, so they must never be held with
    // local interrupts enabled.  Idle polling arrives with IRQs on.
    let _irq_guard = crate::context::IrqSaveGuard::new();

    if !cpu_local(cpu).rq.lock().is_empty() {
        return true;
    }

    let discovered = DISCOVERED_CPUS.load(Ordering::Acquire);
    (0..discovered).any(|donor_index| {
        let donor = CpuId::new(donor_index).expect("invalid donor CPU");
        donor != cpu
            && crate::smp::is_scheduler_active(donor)
            && cpu_local(donor).rq.lock().iter().any(|id| {
                let task = task_ref(*id);
                task.state() == TaskState::Runnable
                    && task.affinity().is_none()
                    && task.migration_disable_depth.load(Ordering::Acquire) == 0
            })
    })
}

fn context_switches_total() -> u64 {
    let discovered = DISCOVERED_CPUS.load(Ordering::Acquire);
    (0..discovered)
        .map(|index| {
            cpu_local(CpuId::new(index).expect("scheduler CPU index is invalid"))
                .context_switches
                .load(Ordering::Relaxed)
        })
        .sum()
}

fn mm_switches_total() -> u64 {
    let discovered = DISCOVERED_CPUS.load(Ordering::Acquire);
    (0..discovered)
        .map(|index| {
            cpu_local(CpuId::new(index).expect("scheduler CPU index is invalid"))
                .mm_switches
                .load(Ordering::Relaxed)
        })
        .sum()
}

fn preemptions_total() -> u64 {
    let discovered = DISCOVERED_CPUS.load(Ordering::Acquire);
    (0..discovered)
        .map(|index| {
            cpu_local(CpuId::new(index).expect("scheduler CPU index is invalid"))
                .preemptions
                .load(Ordering::Relaxed)
        })
        .sum()
}

fn registered_cpu_mask() -> usize {
    let discovered = DISCOVERED_CPUS.load(Ordering::Acquire);
    (0..discovered).fold(0_usize, |mask, index| {
        let local = cpu_local(CpuId::new(index).expect("scheduler CPU index is invalid"));
        if local.current.load(Ordering::Acquire) != NO_CPU
            && local.idle.load(Ordering::Acquire) != NO_CPU
        {
            mask | (1_usize << index)
        } else {
            mask
        }
    })
}

struct Scheduler {
    tasks: Vec<Option<Task>>,
    retired_tasks: Vec<Task>,
    discovered_cpus: usize,
    live_kernel_threads: usize,
    live_user_threads: usize,
}

impl Scheduler {
    fn new(discovered_cpus: usize) -> Self {
        assert!((1..=MAX_CPUS).contains(&discovered_cpus));

        let mut tasks = Vec::with_capacity(MAX_TASKS);
        tasks.push(Some(Task::boot()));
        assert!(tasks.capacity() >= MAX_TASKS);

        for logical in 1..discovered_cpus {
            let cpu = CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
            let stack = KernelStack::allocate().unwrap_or_else(|error| {
                panic!(
                    "unable to allocate idle stack for CPU {}: {error:?}",
                    cpu.get(),
                );
            });
            let id = TaskId(tasks.len());
            tasks.push(Some(Task::idle(id, cpu, stack)));
        }

        let retired_tasks = Vec::with_capacity(MAX_TASKS);
        assert!(retired_tasks.capacity() >= MAX_TASKS);

        Self {
            tasks,
            retired_tasks,
            discovered_cpus,
            live_kernel_threads: 0,
            live_user_threads: 0,
        }
    }

    fn task(&self, id: TaskId) -> &Task {
        self.tasks
            .get(id.0)
            .and_then(Option::as_ref)
            .unwrap_or_else(|| panic!("task {:?} does not exist", id))
    }

    fn task_mut(&mut self, id: TaskId) -> &mut Task {
        self.tasks
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .unwrap_or_else(|| panic!("task {:?} does not exist", id))
    }

    fn allocate_task_id(&self) -> TaskId {
        if let Some((index, _)) = self
            .tasks
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, task)| task.is_none())
        {
            return TaskId(index);
        }

        assert!(
            self.tasks.len() < MAX_TASKS,
            "kernel task table exhausted: capacity={MAX_TASKS}",
        );
        TaskId(self.tasks.len())
    }

    fn choose_target_cpu(&self) -> CpuId {
        (0..self.discovered_cpus)
            .filter_map(CpuId::new)
            .filter(|cpu| crate::smp::is_scheduler_active(*cpu))
            // Count the task already running on each CPU as well as queued
            // work; considering only queue length repeatedly selected CPU0
            // while other CPUs were idle during parallel rustc builds.
            .min_by_key(|cpu| (cpu_runnable_load(*cpu), cpu.get()))
            .expect("scheduler has no active CPU")
    }

    fn spawn(
        &mut self,
        entry: KernelThreadEntry,
        stack: KernelStack,
        affinity: Option<CpuId>,
        queue_hint: Option<CpuId>,
        request_reschedule: bool,
        kind: TaskKind,
    ) -> (TaskId, CpuId) {
        let target = match affinity.or(queue_hint) {
            Some(cpu) => {
                assert!(
                    cpu.get() < self.discovered_cpus,
                    "task target CPU was not discovered",
                );
                assert!(crate::smp::is_online(cpu), "task target CPU is offline");
                assert!(
                    crate::smp::is_scheduler_active(cpu),
                    "task target CPU is not scheduler-active",
                );
                cpu
            }
            None => self.choose_target_cpu(),
        };

        let id = self.allocate_task_id();
        let task = Task::kernel_thread(id, entry, stack, affinity, kind);

        if id.0 == self.tasks.len() {
            self.tasks.push(Some(task));
        } else {
            assert!(self.tasks[id.0].is_none());
            self.tasks[id.0] = Some(task);
        }

        enqueue_task(id, target);
        if request_reschedule {
            cpu_local(target).need_resched.store(true, Ordering::Release);
        }
        if kind.is_counted_kernel_thread() {
            self.live_kernel_threads += 1;
        }
        (id, target)
    }

    fn spawn_user(
        &mut self,
        thread: Arc<crate::process::Thread>,
        exit_visible: Arc<Completion>,
        process_cleanup: Option<ProcessCleanup>,
        stack: KernelStack,
        affinity: Option<CpuId>,
        queue_hint: Option<CpuId>,
    ) -> (TaskId, CpuId) {
        let target = match affinity.or(queue_hint) {
            Some(cpu) => {
                assert!(cpu.get() < self.discovered_cpus);
                assert!(crate::smp::is_scheduler_active(cpu));
                cpu
            }
            None => self.choose_target_cpu(),
        };

        let id = self.allocate_task_id();
        // Keep a user task on the least-loaded CPU chosen at creation. Cargo
        // and rustc expose abundant process/thread parallelism, so balancing
        // at spawn fills every CPU without migrating a live user trap stack.
        // This also keeps architecture per-CPU trap/IRQ state coherent across
        // blocking syscalls and return-to-user anchor reconstruction.
        let task = Task::user_thread(
            id,
            thread,
            exit_visible,
            process_cleanup,
            stack,
            Some(target),
        );
        if id.0 == self.tasks.len() {
            self.tasks.push(Some(task));
        } else {
            assert!(self.tasks[id.0].is_none());
            self.tasks[id.0] = Some(task);
        }

        // A CLONE_THREAD can join the Process before its scheduler task is
        // inserted.  Recheck after insertion while holding SCHEDULER so a
        // concurrent exit_group scan cannot leave that late clone alive.
        let late_forced_exit = self.task(id).user_thread.as_ref().and_then(|thread| {
            let process = thread.process();
            process
                .group_exit_status()
                .or_else(|| process.exec_in_progress().then_some(0))
                .map(|status| (Arc::clone(thread), status))
        });
        if let Some((thread, status)) = late_forced_exit {
            thread.request_forced_exit(status);
        }
        enqueue_task(id, target);
        cpu_local(target).need_resched.store(true, Ordering::Release);
        self.live_user_threads = self
            .live_user_threads
            .checked_add(1)
            .expect("live user-thread counter overflowed");
        USER_TASKS_SPAWNED.fetch_add(1, Ordering::Relaxed);
        (id, target)
    }

    fn prepare_block(&mut self, cpu: CpuId, queue: &WaitQueue) -> ContextSwitch {
        let channel = queue.channel();
        assert_ne!(channel, 0, "wait channel zero is reserved");
        {
            let local = cpu_local(cpu);
            assert!(local.pending().is_none(), "nested switch attempted");
            assert_eq!(local.irq_depth, 0, "IRQ context attempted to block");
        }
        let previous = current_of(cpu);
        let previous_task = self.task(previous);
        assert_eq!(
            previous_task.preempt_disable_depth.load(Ordering::Acquire),
            0,
            "preemption-disabled task attempted to block",
        );
        assert_eq!(previous_task.state(), TaskState::Running(cpu));
        assert!(
            !previous_task.kind.is_idle(),
            "idle task attempted to block on a wait queue",
        );
        assert!(
            previous_task.wait_channel.is_none(),
            "task attempted to join a second wait queue: task={previous:?} channel={:?}",
            previous_task.wait_channel,
        );
        assert!(previous_task.wait_queue_address.is_none());
        assert!(
            previous_task.wait_prev.is_none() && previous_task.wait_next.is_none(),
            "running task retained stale intrusive wait links: task={previous:?}",
        );
        assert!(
            !previous_task.wake_after_switch,
            "running task retained a stale wake claim: task={previous:?}",
        );
        #[cfg(debug_assertions)]
        assert_eq!(
            crate::tracked_spin::preemptible_lock_depth(cpu),
            0,
            "task attempted to block while holding a preemptible tracked lock: cpu={} task={previous:?}",
            cpu.get(),
        );

        let next = dequeue_next_task(cpu).unwrap_or_else(|| idle_of(cpu));
        assert_ne!(previous, next);
        {
            let task = self.task_mut(previous);
            task.store_state(TaskState::SwitchingOut(cpu));
            task.wake_after_switch = false;
        }
        self.link_waiter(queue, previous, channel);
        activate_next_task(next, cpu);
        switch_mm_irqs_off(cpu, previous, next);
        {
            let local = cpu_local_mut(cpu);
            local.store_pending(Some(PendingSwitch {
                previous,
                next,
                disposition: SwitchDisposition::Block,
            }));
            local.context_switches.fetch_add(1, Ordering::Relaxed);
            local
                .last_switch
                .store(crate::arch::time::counter(), Ordering::Relaxed);
        local
            .last_switch
            .store(crate::arch::time::counter(), Ordering::Relaxed);
        }
        context_pair(previous, next)
    }

    fn prepare_exit(&mut self, cpu: CpuId) -> ContextSwitch {
        assert!(
            cpu_local(cpu).pending().is_none(),
            "CPU attempted a nested context switch",
        );

        let previous = current_of(cpu);
        assert_eq!(self.task(previous).state(), TaskState::Running(cpu));
        assert!(
            !self.task(previous).kind.is_idle(),
            "idle task attempted to exit",
        );

        {
            let previous_task = self.task(previous);
            assert!(
                previous_task.wait_channel.is_none(),
                "exiting task retained wait-channel ownership: task={previous:?} channel={:?}",
                previous_task.wait_channel,
            );
            assert!(previous_task.wait_queue_address.is_none());
            assert!(
                !previous_task.wake_after_switch,
                "exiting task retained a pending wake claim: task={previous:?}",
            );
        }
        #[cfg(debug_assertions)]
        assert_eq!(
            crate::tracked_spin::preemptible_lock_depth(cpu),
            0,
            "task attempted to exit while holding a preemptible tracked lock: cpu={} task={previous:?}",
            cpu.get(),
        );
        let next = dequeue_next_task(cpu).unwrap_or_else(|| idle_of(cpu));
        assert_ne!(previous, next);

        self.task_mut(previous)
            .store_state(TaskState::SwitchingOut(cpu));
        activate_next_task(next, cpu);
        switch_mm_irqs_off(cpu, previous, next);
        {
            let local = cpu_local_mut(cpu);
            local.store_pending(Some(PendingSwitch {
                previous,
                next,
                disposition: SwitchDisposition::Exit,
            }));
            local.context_switches.fetch_add(1, Ordering::Relaxed);
            local
                .last_switch
                .store(crate::arch::time::counter(), Ordering::Relaxed);
        local
            .last_switch
            .store(crate::arch::time::counter(), Ordering::Relaxed);
        }
        context_pair(previous, next)
    }

    /// Switch-tail commit for Block/Exit: both dispositions mutate
    /// registry-owned task state and must run under REGISTRY.
    fn complete_registry_switch(
        &mut self,
        cpu: CpuId,
        pending: PendingSwitch,
        running_sp: usize,
    ) -> (CompletedSwitch, bool) {
        assert_eq!(
            current_of(cpu),
            pending.previous,
            "scheduler current changed before the hardware stack switch committed",
        );
        assert_eq!(
            self.task(pending.next).state(),
            TaskState::Running(cpu),
            "incoming task was not Running before switch commit",
        );
        #[cfg(not(debug_assertions))]
        let _ = running_sp;
        #[cfg(debug_assertions)]
        assert!(
            self.task(pending.next).stack.is_none()
                || self.task(pending.next).stack_contains(running_sp),
            "incoming hardware SP is outside the selected task stack: task={:?} sp={running_sp:#x}",
            pending.next,
        );
        cpu_local(cpu)
            .current
            .store(pending.next.raw(), Ordering::Release);
        assert_eq!(
            self.task(pending.previous).state(),
            TaskState::SwitchingOut(cpu),
        );

        match pending.disposition {
            SwitchDisposition::Yield => unreachable!("yield resolved without the registry"),
            SwitchDisposition::Block => {
                let wake_after_switch = self.task(pending.previous).wake_after_switch;

                if wake_after_switch {
                    {
                        let task = self.task_mut(pending.previous);
                        assert!(
                            task.wait_channel.is_some(),
                            "claimed switching waiter lost its wait channel",
                        );
                        task.wake_after_switch = false;
                        task.wait_channel = None;
                        task.wait_queue_address = None;
                        task.store_state(TaskState::Runnable);
                    }
                    enqueue_task(pending.previous, cpu);
                    // The wake IPI may have arrived while local interrupts were
                    // disabled for the context switch. Preserve the scheduling
                    // request in software so progress does not depend on an
                    // interrupt-controller edge being replayed.
                    cpu_local(cpu).need_resched.store(true, Ordering::Release);
                } else {
                    let task = self.task_mut(pending.previous);
                    assert!(
                        task.wait_channel.is_some(),
                        "blocking task reached schedule-tail without a wait channel",
                    );
                    task.store_state(TaskState::Blocked);
                }
            }
            SwitchDisposition::Exit => {
                self.task_mut(pending.previous)
                    .store_state(TaskState::Exited);
                if self.task(pending.previous).kind.is_counted_kernel_thread() {
                    self.live_kernel_threads = self
                        .live_kernel_threads
                        .checked_sub(1)
                        .expect("live kernel-thread counter underflowed");
                }
                if matches!(self.task(pending.previous).kind, TaskKind::UserThread) {
                    self.live_user_threads = self
                        .live_user_threads
                        .checked_sub(1)
                        .expect("live user-thread counter underflowed");
                }

                let mut task = self.tasks[pending.previous.0]
                    .take()
                    .expect("exited task disappeared before reclamation");
                assert_eq!(task.id, pending.previous);
                assert_eq!(task.state(), TaskState::Exited);
                let exit_visible = if matches!(task.kind, TaskKind::UserThread) {
                    Some(ExitVisible {
                        completion: task
                            .exit_visible
                            .take()
                            .expect("exited user task lost its exit-visible completion"),
                    })
                } else {
                    None
                };
                // M15-A: grow the deferred reclamation queue instead of
                // panicking during SMP verifier/user-exit bursts. Linux-like
                // task reclamation must tolerate transient exit bursts until
                // the reaper catches up.
                if self.retired_tasks.len() == self.retired_tasks.capacity() {
                    self.retired_tasks
                        .try_reserve(MAX_TASKS)
                        .expect("unable to grow retired task queue");
                }
                self.retired_tasks.push(task);
                TASKS_RETIRED.fetch_add(1, Ordering::Relaxed);
                // Publish the flush/barrier lifetime before making the task
                // visible to the reaper.  The REGISTRY lock prevents a consumer
                // from detaching the task between these two publications.
                RETIRED_OUTSTANDING
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                        value.checked_add(1)
                    })
                    .expect("retired outstanding counter overflowed");
                RETIRED_BACKLOG
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                        value.checked_add(1)
                    })
                    .expect("retired backlog counter overflowed");
                return (
                    CompletedSwitch {
                        retired_task_added: true,
                        exit_visible,
                    },
                    self.task(pending.next).kind.is_idle(),
                );
            }
        }

        (
            CompletedSwitch {
                retired_task_added: false,
                exit_visible: None,
            },
            self.task(pending.next).kind.is_idle(),
        )
    }

    fn take_retired_task(&mut self) -> Option<Task> {
        let task = self.retired_tasks.pop()?;
        RETIRED_BACKLOG
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_sub(1)
            })
            .expect("retired backlog counter underflowed");
        Some(task)
    }

    #[cfg(debug_assertions)]
    fn clear_current_affinity(&mut self, cpu: CpuId) {
        let current = current_of(cpu);
        let task = self.task(current);

        assert_eq!(task.state(), TaskState::Running(cpu));
        assert!(
            !task.kind.is_idle(),
            "idle task affinity must remain fixed to its CPU",
        );
        task.store_affinity(None);
    }

    #[cfg(debug_assertions)]
    fn task_is_runnable_on(&self, id: TaskId, cpu: CpuId) -> bool {
        let task = self.task(id);
        task.state() == TaskState::Runnable && task.queued_on() == Some(cpu)
    }

    #[cfg(debug_assertions)]
    fn migrate_runnable_task(&mut self, id: TaskId, target: CpuId) -> CpuId {
        assert!(
            crate::smp::is_scheduler_active(target),
            "migration target CPU is not scheduler-active"
        );

        let (source, has_run, affinity) = {
            let task = self.task(id);
            assert_eq!(task.state(), TaskState::Runnable);
            (
                task.queued_on()
                    .expect("runnable migration task is not queued"),
                task.has_run.load(Ordering::Acquire),
                task.affinity(),
            )
        };

        assert!(has_run, "migration test requires an already-run task");
        assert!(
            affinity.is_none() || affinity == Some(source),
            "task is pinned away from its migration source",
        );
        assert_ne!(source, target, "migration source and target are identical");

        let removed = {
            let mut queue = cpu_local(source).rq.lock();
            let position = queue
                .iter()
                .position(|candidate| *candidate == id)
                .expect("migration task disappeared from its source run queue");
            queue
                .remove(position)
                .expect("migration task removal failed")
        };
        assert_eq!(removed, id);

        {
            let task = self.task(id);
            assert_eq!(task.queued_on(), Some(source));
            task.store_queued_on(None);
            // Retarget affinity and queue ownership in the same REGISTRY
            // critical section. This prevents a work-stealing CPU from racing
            // the explicit hand-off between source removal and target enqueue.
            task.store_affinity(Some(target));
        }

        enqueue_task(id, target);
        cpu_local(target).need_resched.store(true, Ordering::Release);
        source
    }

    fn register_secondary(&mut self, cpu: CpuId) {
        assert_ne!(cpu, CpuId::BOOT);
        assert!(cpu.get() < self.discovered_cpus);
        assert_eq!(
            crate::smp::cpu_state(cpu),
            crate::smp::CpuState::Starting,
            "secondary scheduler registration occurred in the wrong lifecycle state",
        );
        assert_eq!(
            cpu_local(cpu).current.load(Ordering::Acquire),
            NO_CPU,
            "secondary CPU registered twice",
        );

        let idle = idle_of(cpu);
        assert_eq!(self.task(idle).state(), TaskState::Idle(cpu));
        self.task_mut(idle).store_state(TaskState::Running(cpu));
        self.task_mut(idle).has_run.store(true, Ordering::Release);
        cpu_local(cpu)
            .current
            .store(idle.raw(), Ordering::Release);
    }

    fn activate_secondary(&mut self, cpu: CpuId) {
        assert_ne!(cpu, CpuId::BOOT);
        assert!(cpu.get() < self.discovered_cpus);
        assert_eq!(
            crate::smp::cpu_state(cpu),
            crate::smp::CpuState::SchedulerRegistered,
            "secondary activation occurred in the wrong lifecycle state",
        );

        let idle = idle_of(cpu);
        assert_eq!(current_of(cpu), idle);
        assert_eq!(self.task(idle).state(), TaskState::Running(cpu));
        assert!(
            cpu_local(cpu).pending().is_none(),
            "CPU became active during a switch",
        );
    }

    fn retired_task_count(&self) -> usize {
        self.retired_tasks.len()
    }

    fn secondary_idle_context(&self, cpu: CpuId) -> *const crate::arch::task::Context {
        let idle = idle_of(cpu);
        assert_eq!(current_of(cpu), idle);
        task_ref(idle).context_ptr()
    }

    fn current_entry(&self, cpu: CpuId) -> KernelThreadEntry {
        task_ref(current_of(cpu))
            .entry
            .expect("current task is not a kernel thread")
    }

    fn current_user_thread(&self, cpu: CpuId) -> Option<Arc<crate::process::Thread>> {
        task_ref(current_of(cpu)).user_thread.as_ref().cloned()
    }

    fn request_process_thread_exit(
        &mut self,
        process: crate::process::ProcessId,
        except: crate::process::ThreadId,
        status: isize,
    ) -> usize {
        let mut wake = Vec::new();
        let mut target_mask = 0_usize;

        for index in 0..self.tasks.len() {
            let Some(task) = self.tasks[index].as_ref() else {
                continue;
            };
            let Some(thread) = task.user_thread.as_ref() else {
                continue;
            };
            if thread.process().id() != process || thread.id() == except {
                continue;
            }

            thread.request_forced_exit(status);
            match task.state() {
                TaskState::Blocked => {
                    if let Some(address) = task.wait_queue_address {
                        wake.push((task.id, address));
                    }
                }
                TaskState::SwitchingOut(cpu) if !task.wake_after_switch => {
                    if let Some(address) = task.wait_queue_address {
                        wake.push((task.id, address));
                    }
                    target_mask |= 1_usize << cpu.get();
                }
                TaskState::Running(cpu) => target_mask |= 1_usize << cpu.get(),
                TaskState::Runnable => {
                    if let Some(cpu) = task.queued_on() {
                        target_mask |= 1_usize << cpu.get();
                    }
                }
                TaskState::SwitchingOut(cpu) => target_mask |= 1_usize << cpu.get(),
                TaskState::Idle(_) | TaskState::Exited => {}
            }
        }

        for (task, address) in wake {
            // SAFETY: a linked waiter keeps the WaitQueue alive until it is
            // unlinked. The scheduler lock serializes this lookup with normal
            // wakeup and switch-tail queue removal.
            let queue = unsafe { &*(address as *const WaitQueue) };
            let (_, targets) = self.wake_waiters(queue, 1, Some(task));
            target_mask |= targets;
        }
        target_mask
    }

    #[cfg(debug_assertions)]
    fn current_stack_contains(&self, cpu: CpuId, address: usize) -> bool {
        task_ref(current_of(cpu)).stack_contains(address)
    }

    fn link_waiter(&mut self, queue: &WaitQueue, id: TaskId, channel: usize) {
        let mut list = queue.waiters.lock();
        assert!(list.count < MAX_TASKS, "wait queue capacity exhausted");

        {
            let task = self.task(id);
            assert!(matches!(task.state(), TaskState::SwitchingOut(_)));
            assert!(task.wait_channel.is_none());
            assert!(task.wait_queue_address.is_none());
            assert!(task.wait_prev.is_none() && task.wait_next.is_none());
        }

        let previous_tail = list.tail;
        if let Some(tail) = previous_tail {
            let tail_task = self.task_mut(tail);
            assert!(
                tail_task.wait_next.is_none(),
                "wait queue tail had a successor"
            );
            tail_task.wait_next = Some(id);
        } else {
            assert!(list.head.is_none(), "empty wait queue retained a head");
            list.head = Some(id);
        }

        {
            let task = self.task_mut(id);
            task.wait_channel = Some(channel);
            task.wait_queue_address = Some(queue as *const WaitQueue as usize);
            task.wait_prev = previous_tail;
            task.wait_next = None;
        }
        list.tail = Some(id);
        list.count = list.count.checked_add(1).expect("waiter count overflowed");
    }

    fn waiter_is_linked(&self, list: &wait_queue::WaitList, id: TaskId, channel: usize) -> bool {
        let task = self.task(id);
        task.wait_channel == Some(channel)
            && (list.head == Some(id)
                || list.tail == Some(id)
                || task.wait_prev.is_some()
                || task.wait_next.is_some())
    }

    fn unlink_waiter_locked(
        &mut self,
        list: &mut wait_queue::WaitList,
        id: TaskId,
        channel: usize,
    ) {
        assert!(
            self.waiter_is_linked(list, id, channel),
            "waiter was unlinked twice or belonged to another queue",
        );
        let (previous, next) = {
            let task = self.task(id);
            assert_eq!(
                task.wait_channel,
                Some(channel),
                "waiter belonged to a different queue",
            );
            (task.wait_prev, task.wait_next)
        };

        if let Some(previous) = previous {
            let previous_task = self.task_mut(previous);
            assert_eq!(previous_task.wait_next, Some(id));
            previous_task.wait_next = next;
        } else {
            assert_eq!(list.head, Some(id));
            list.head = next;
        }

        if let Some(next) = next {
            let next_task = self.task_mut(next);
            assert_eq!(next_task.wait_prev, Some(id));
            next_task.wait_prev = previous;
        } else {
            assert_eq!(list.tail, Some(id));
            list.tail = previous;
        }

        {
            let task = self.task_mut(id);
            task.wait_prev = None;
            task.wait_next = None;
        }
        list.count = list.count.checked_sub(1).expect("waiter count underflowed");
        assert_eq!(list.head.is_none(), list.tail.is_none());
        assert_eq!(list.count == 0, list.head.is_none());
    }

    fn wake_waiters(
        &mut self,
        queue: &WaitQueue,
        maximum: usize,
        target: Option<TaskId>,
    ) -> (usize, usize) {
        assert!((1..=MAX_TASKS).contains(&maximum));
        let channel = queue.channel();
        // Phase 1 (WaitQueue lock held): unlink waiters and transition their
        // state.  Target selection and Runnable publication happen in phase 2
        // after the waiters lock is dropped, because choose_target_cpu and the
        // rq locks (rank 25) must never run under a WaitQueue lock (rank 30).
        let mut target_mask = 0;
        let mut count = 0;
        let mut woken_list: Vec<(TaskId, bool)> = Vec::new();

        {
            let mut list = queue.waiters.lock();

            while count < maximum {
                let id = match target {
                    Some(target) => {
                        if count != 0 || !self.waiter_is_linked(&list, target, channel) {
                            break;
                        }
                        target
                    }
                    None => match list.head {
                        Some(head) => head,
                        None => break,
                    },
                };

                self.unlink_waiter_locked(&mut list, id, channel);
                match self.task(id).state() {
                    TaskState::Blocked => {
                        let is_user_thread = matches!(self.task(id).kind, TaskKind::UserThread);
                        let task = self.task_mut(id);
                        assert!(!task.wake_after_switch);
                        assert!(task.wait_prev.is_none() && task.wait_next.is_none());
                        task.wait_channel = None;
                        task.wait_queue_address = None;
                        task.store_state(TaskState::Runnable);
                        woken_list.push((id, is_user_thread));
                        count += 1;
                    }
                    TaskState::SwitchingOut(cpu) => {
                        let task = self.task_mut(id);
                        assert!(!task.wake_after_switch, "waiter was claimed twice");
                        assert!(task.wait_prev.is_none() && task.wait_next.is_none());
                        // The queue link is already gone, but switch-tail still
                        // owns wait_channel until the old stack is no longer live.
                        task.wake_after_switch = true;
                        target_mask |= 1_usize << cpu.get();
                        count += 1;
                    }
                    state => panic!("invalid waiter state during wakeup: {state:?}"),
                }

                if target.is_some() {
                    break;
                }
            }
        }

        // Phase 2 (WaitQueue lock released): choose the landing CPU and
        // publish Runnable tasks into the target runqueues.  The whole wakeup
        // remains serialized by REGISTRY, so no other path can observe the
        // unqueued Runnable window.
        for (id, is_user_thread) in woken_list {
            let target_cpu = if let Some(pinned) = self.task(id).pinned_cpu() {
                // A migration-pinned task must resume on the CPU recorded at
                // pin time, regardless of queue balance.
                pinned
            } else if is_user_thread {
                // A blocked user task owns no live CPU/trap anchor.
                // Rebalance only at this safe boundary; running and
                // timer-preempted user tasks remain fixed.
                self.choose_target_cpu()
            } else {
                self.task(id)
                    .affinity()
                    .unwrap_or_else(|| self.choose_target_cpu())
            };
            if is_user_thread {
                self.task(id).store_affinity(Some(target_cpu));
            }
            enqueue_task(id, target_cpu);
            cpu_local(target_cpu)
                .need_resched
                .store(true, Ordering::Release);
            target_mask |= 1_usize << target_cpu.get();
        }

        (count, target_mask)
    }

    #[cfg(debug_assertions)]
    fn waiter_debug_state(&self, channel: usize) -> WaiterDebugState {
        let mut state = WaiterDebugState::default();

        for task in self.tasks.iter().flatten() {
            if task.wait_channel != Some(channel) {
                continue;
            }

            match task.state() {
                TaskState::Blocked => {
                    assert!(!task.wake_after_switch);
                    state.blocked += 1;
                }
                TaskState::SwitchingOut(_) if task.wake_after_switch => {
                    state.claimed_switching += 1;
                }
                TaskState::SwitchingOut(_) => {
                    state.switching += 1;
                }
                other => panic!(
                    "task retained a wait channel in an invalid state: task={:?} state={other:?}",
                    task.id,
                ),
            }
        }

        state
    }

    fn assert_user_mm_quiescent(&self) {
        assert_eq!(self.live_user_threads, 0, "M9-B leaked a live user task");
        assert!(
            self.tasks
                .iter()
                .flatten()
                .all(|task| !matches!(task.kind, TaskKind::UserThread)),
            "M9-B retained a user task in the scheduler table",
        );
        assert!(
            self.retired_tasks
                .iter()
                .filter(|task| matches!(task.kind, TaskKind::UserThread))
                .all(|task| { task.state() == TaskState::Exited && task.exit_visible.is_none() }),
            "M9-B retained a non-retired user task in the reaper queue",
        );
        for index in 0..self.discovered_cpus {
            assert!(
                cpu_local(CpuId::new(index).expect("scheduler CPU index is invalid"))
                    .loaded_mm()
                    .is_none(),
                "M9-B CPU {index} retained a loaded user MM",
            );
        }
    }
}

static SCHEDULER: IrqSpinLock<Option<Scheduler>> =
    IrqSpinLock::new_with_class(None, LockClass::new("scheduler", LockRank::Scheduler, 1));
static TASK_REAPER_QUEUE: WaitQueue = WaitQueue::new();

// Queue-empty and reclamation-complete are different states: after a task is
// detached from Scheduler::retired_tasks its guarded stack may still be in the
// VM/TLB destruction path.  This wait queue implements the quiescent verifier
// barrier without holding a spin lock across resource destruction.
static TASK_REAPER_DRAINED: WaitQueue = WaitQueue::new();
// Number of tasks still linked in Scheduler::retired_tasks.
static RETIRED_BACKLOG: AtomicUsize = AtomicUsize::new(0);
// Number of retired tasks whose resources are not fully destroyed yet.  This
// includes both queued tasks and tasks currently owned by a reaper worker.
static RETIRED_OUTSTANDING: AtomicUsize = AtomicUsize::new(0);
static USER_TASKS_SPAWNED: AtomicU64 = AtomicU64::new(0);
static USER_TASKS_EXIT_VISIBLE: AtomicU64 = AtomicU64::new(0);
static TASKS_RETIRED: AtomicU64 = AtomicU64::new(0);
static TASKS_RECLAIMED: AtomicU64 = AtomicU64::new(0);
static EXIT_VISIBLE_WAIT_BEGIN: AtomicU64 = AtomicU64::new(0);
static EXIT_VISIBLE_WAIT_END: AtomicU64 = AtomicU64::new(0);
static IDLE_ENTERS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static IDLE_EXITS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TaskLifecycleSnapshot {
    pub tasks_spawned: u64,
    pub tasks_exit_visible: u64,
    pub tasks_retired: u64,
    pub tasks_reclaimed: u64,
    pub join_wait_begin: u64,
    pub join_wait_end: u64,
    pub retired_backlog: usize,
    pub retired_outstanding: usize,
    pub live_user_threads: usize,
    pub live_kernel_threads: usize,
    pub live_processes: usize,
    pub live_threads: usize,
}

pub(crate) fn task_lifecycle_snapshot() -> TaskLifecycleSnapshot {
    let (live_user_threads, live_kernel_threads) = {
        let slot = SCHEDULER.lock();
        let scheduler = slot.as_ref().expect("kernel scheduler is not initialized");
        (scheduler.live_user_threads, scheduler.live_kernel_threads)
    };
    TaskLifecycleSnapshot {
        tasks_spawned: USER_TASKS_SPAWNED.load(Ordering::Acquire),
        tasks_exit_visible: USER_TASKS_EXIT_VISIBLE.load(Ordering::Acquire),
        tasks_retired: TASKS_RETIRED.load(Ordering::Acquire),
        tasks_reclaimed: TASKS_RECLAIMED.load(Ordering::Acquire),
        join_wait_begin: EXIT_VISIBLE_WAIT_BEGIN.load(Ordering::Acquire),
        join_wait_end: EXIT_VISIBLE_WAIT_END.load(Ordering::Acquire),
        retired_backlog: RETIRED_BACKLOG.load(Ordering::Acquire),
        retired_outstanding: RETIRED_OUTSTANDING.load(Ordering::Acquire),
        live_user_threads,
        live_kernel_threads,
        live_processes: crate::process::live_process_count(),
        live_threads: crate::process::live_thread_count(),
    }
}

pub(crate) fn print_task_lifecycle_summary() {
    let snapshot = task_lifecycle_snapshot();
    crate::println!(
        "task-lifecycle-summary spawned={} visible={} retired={} reclaimed={} join_begin={} join_end={} backlog={} outstanding={} live_user={} live_kernel={} live_processes={} live_threads={}",
        snapshot.tasks_spawned,
        snapshot.tasks_exit_visible,
        snapshot.tasks_retired,
        snapshot.tasks_reclaimed,
        snapshot.join_wait_begin,
        snapshot.join_wait_end,
        snapshot.retired_backlog,
        snapshot.retired_outstanding,
        snapshot.live_user_threads,
        snapshot.live_kernel_threads,
        snapshot.live_processes,
        snapshot.live_threads,
    );
}

/// Lightweight heartbeat string for build watchdog: just the live thread/process
/// counts so we can track progress without flooding the serial log.
pub(crate) fn thread_count_summary() -> alloc::string::String {
    let live_processes = crate::process::live_process_count();
    let live_threads = crate::process::live_thread_count();
    let live_user: usize = {
        let slot = SCHEDULER.lock();
        slot.as_ref().map(|s| s.live_user_threads).unwrap_or(0)
    };
    alloc::format!(
        "live_user={} live_processes={} live_threads={}",
        live_user,
        live_processes,
        live_threads,
    )
}

pub(crate) fn print_lifecycle_stress_progress(label: &str, iteration: usize) {
    let cpu = crate::smp::current_cpu_id();
    let current = current_of(cpu);
    let boot_sp = task_ref(TaskId(0)).context().saved_stack_pointer();
    let current_sp = task_ref(current).context().saved_stack_pointer();
    crate::println!(
        "G2_PROGRESS {} iteration={} cpu={} current={:?} boot_saved_sp={:#x} current_saved_sp={:#x} free_pages={}",
        label,
        iteration,
        cpu.get(),
        current,
        boot_sp,
        current_sp,
        crate::page_alloc::total_free_pages().unwrap_or(0),
    );
}

pub(crate) fn print_task_debug_dump() {
    // smp-pcpu-v1: the task-table snapshot still runs under REGISTRY (it
    // serializes spawn/exit slot writes); the per-CPU half is lockless —
    // atomics plus a try_lock probe of each runqueue so a wedged owner
    // cannot deadlock the panic diagnostic.
    let tasks = {
        let slot = SCHEDULER.lock();
        let scheduler = slot.as_ref().expect("kernel scheduler is not initialized");
        scheduler
            .tasks
            .iter()
            .flatten()
            .map(|task| {
                (
                    task.id,
                    task.kind,
                    task.state(),
                    task.wait_channel,
                    task.queued_on(),
                    task.user_thread.as_ref().map(|thread| {
                        (
                            thread.process().id().get(),
                            thread.id().get(),
                            thread.schedule_count(),
                            thread.visited_cpu_mask(),
                            thread.forced_exit_status(),
                        )
                    }),
                )
            })
            .collect::<Vec<_>>()
    };
    let cpus = {
        let discovered = DISCOVERED_CPUS.load(Ordering::Acquire);
        (0..discovered)
            .map(|index| {
                let cpu = CpuId::new(index).expect("scheduler CPU index is invalid");
                let local = cpu_local(cpu);
                let runnable = local
                    .rq
                    .try_lock()
                    .map_or(-1, |queue| queue.len() as isize);
                (
                    index,
                    TaskId::from_raw(local.current.load(Ordering::Acquire)),
                    runnable,
                    local.need_resched.load(Ordering::Acquire),
                    local.pending(),
                )
            })
            .collect::<Vec<_>>()
    };
    crate::println!("sudoos-diag: task-debug-dump begin");
    for (id, kind, state, wait_channel, queued_on, user) in tasks {
        crate::println!(
            "sudoos-diag: task id={:?} kind={:?} state={:?} wait={:?} queued={:?} user={:?}",
            id,
            kind,
            state,
            wait_channel,
            queued_on,
            user,
        );
    }
    for (cpu, current, runnable, need_resched, pending) in cpus {
        crate::println!(
            "sudoos-diag: cpu={} current={:?} runnable={} need_resched={} pending={:?}",
            cpu,
            current,
            runnable,
            need_resched,
            pending,
        );
    }
    print_task_lifecycle_summary();
    crate::println!("sudoos-diag: task-debug-dump end");
}

pub fn initialize() {
    let discovered = crate::smp::discovered_cpu_count();
    let mut scheduler = Scheduler::new(discovered);

    {
        let mut slot = SCHEDULER.lock();

        assert!(slot.is_none(), "kernel scheduler was initialized twice");
        // smp-pcpu-v1 publication point: the lockless fast paths are allowed
        // to execute the moment the task table goes live.  The idle-task IDs
        // are stable for the lifetime of the registry (task 0 is the boot
        // idle, tasks 1..discovered are the secondary idles).
        TASKS_BASE.store(scheduler.tasks.as_mut_ptr(), Ordering::Release);
        DISCOVERED_CPUS.store(discovered, Ordering::Release);
        let boot = cpu_local(CpuId::BOOT);
        boot.current.store(TaskId(0).raw(), Ordering::Release);
        boot.idle.store(TaskId(0).raw(), Ordering::Release);
        for logical in 1..discovered {
            let cpu = CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
            cpu_local(cpu)
                .idle
                .store(TaskId(logical).raw(), Ordering::Release);
        }
        *slot = Some(scheduler);
    }

    crate::smp::mark_boot_scheduler_registered();
    crate::smp::mark_current_scheduler_active();

    spawn_system_thread(task_reaper_main, Some(CpuId::BOOT), Some(CpuId::BOOT));

    #[cfg(debug_assertions)]
    wait_queue::verify_local();
    crate::println!("kernel scheduler:");
    crate::println!("  policy          : preemptive per-CPU FIFO round-robin");
    crate::println!("  kernel stack    : 64 KiB plus guard pages");
    crate::println!("  bootstrap CPUs  : 1");
    crate::println!("  configured CPUs : {}", discovered);
    crate::println!(
        "  timeslice       : {} timer ticks",
        DEFAULT_TIME_SLICE_TICKS
    );
    crate::println!("  wait queues     : blocking wakeup enabled");
    crate::println!("  task reaper     : dedicated kernel thread");
    crate::println!("  migration       : runnable tasks may move across CPUs");
}

pub fn irq_enter() {
    let cpu = crate::smp::current_cpu_id();
    if !crate::smp::is_online(cpu) || !scheduler_ready() {
        return;
    }
    // smp-pcpu-v1: IRQ depth is truly-local CpuLocal state.
    let local = cpu_local_mut(cpu);
    local.irq_depth = local
        .irq_depth
        .checked_add(1)
        .expect("IRQ nesting counter overflowed");
}

pub fn irq_exit() {
    let cpu = crate::smp::current_cpu_id();
    if !crate::smp::is_online(cpu) || !scheduler_ready() {
        return;
    }
    let should_preempt = {
        let local = cpu_local_mut(cpu);
        local.irq_depth = local
            .irq_depth
            .checked_sub(1)
            .expect("IRQ nesting counter underflowed");
        local.irq_depth == 0
            && task_ref(current_of(cpu))
                .preempt_disable_depth
                .load(Ordering::Acquire)
                == 0
            && local.need_resched.load(Ordering::Acquire)
    };

    if should_preempt {
        preempt_schedule_irq();
    }
}

pub fn on_timer_ticks(ticks: u64) {
    if ticks == 0 {
        return;
    }
    let cpu = crate::smp::current_cpu_id();
    if !scheduler_ready() {
        return;
    }
    timer_ticks_local(cpu, ticks);
}

pub fn request_reschedule_local() {
    let cpu = crate::smp::current_cpu_id();
    if scheduler_ready() && crate::smp::is_scheduler_active(cpu) {
        cpu_local(cpu).need_resched.store(true, Ordering::Release);
    }
}

pub(crate) fn request_process_thread_exit(
    process: crate::process::ProcessId,
    except: crate::process::ThreadId,
    status: isize,
) {
    let targets = {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("scheduler is not initialized")
            .request_process_thread_exit(process, except, status)
    };
    send_wakeup_ipis(targets);
}

fn request_reschedule_on(cpu: CpuId) {
    if scheduler_ready() && crate::smp::is_scheduler_active(cpu) {
        cpu_local(cpu).need_resched.store(true, Ordering::Release);
    }

    if cpu != crate::smp::current_cpu_id() {
        crate::smp::send_ipi(cpu);
    }
}

pub fn preempt_disable() -> TaskId {
    preempt_disable_local()
}

pub fn preempt_enable() {
    let task = current_task_id();
    preempt_enable_task(task);
}

fn preempt_enable_task(task: TaskId) {
    preempt_enable_local(task);
}

pub fn preempt_count() -> usize {
    let cpu = crate::smp::current_cpu_id();
    task_ref(current_of(cpu))
        .preempt_disable_depth
        .load(Ordering::Acquire)
}

pub(crate) fn current_task_diagnostic() -> (TaskId, &'static str, usize) {
    // Panic-path diagnostic: repair identity from the executing stack before
    // reporting, since an RV user trap anchor may still name a stale CPU.
    repair_current_cpu_from_stack();
    let cpu = crate::smp::current_cpu_id();
    let id = current_of(cpu);
    let task = task_ref(id);
    let kind = match task.kind {
        TaskKind::Idle(_) => "idle",
        TaskKind::KernelThread => "kernel",
        TaskKind::SystemThread => "system",
        TaskKind::UserThread => "user",
    };
    (id, kind, task.preempt_disable_depth.load(Ordering::Acquire))
}

pub fn irq_depth() -> usize {
    let cpu = crate::smp::current_cpu_id();
    cpu_local(cpu).irq_depth
}

/// §10 diagnostic mirror: records the last known local interrupt state.
/// Updated at IrqSaveGuard boundaries, trap entries/exits, idle transitions
/// and explicit interrupt state changes. Read cross-CPU only from panic paths.
pub(crate) fn note_irq_state(enabled: bool) {
    // Early boot runs before tp carries a valid logical CPU id; the mirror is
    // meaningless then and must not fault.
    let raw = crate::arch::smp::current_cpu_id();
    if raw >= crate::smp::MAX_CPUS {
        return;
    }
    let cpu = CpuId::new(raw).expect("guarded logical CPU ID exceeds MAX_CPUS");
    let local = cpu_local(cpu);
    local.irq_mirror.store(enabled as u8, Ordering::Relaxed);
    local
        .irq_mirror_at
        .store(crate::arch::time::counter(), Ordering::Relaxed);
}

pub(crate) fn note_trap_entry() {
    let raw = crate::arch::smp::current_cpu_id();
    if raw >= crate::smp::MAX_CPUS {
        return;
    }
    let cpu = CpuId::new(raw).expect("guarded logical CPU ID exceeds MAX_CPUS");
    cpu_local(cpu).in_trap_depth.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_trap_exit() {
    let raw = crate::arch::smp::current_cpu_id();
    if raw >= crate::smp::MAX_CPUS {
        return;
    }
    let cpu = CpuId::new(raw).expect("guarded logical CPU ID exceeds MAX_CPUS");
    let local = cpu_local(cpu);
    // A switch inside a trap carries the trap continuation to another CPU:
    // the entry was counted on the source CPU and this exit runs on the
    // destination. Saturate and record the imbalance instead of panicking —
    // this is a diagnostic mirror, and the migration is by design.
    let previous = local.in_trap_depth.load(Ordering::Relaxed);
    if previous == 0 {
        local.trap_imbalance.fetch_add(1, Ordering::Relaxed);
        return;
    }
    local.in_trap_depth.store(previous - 1, Ordering::Relaxed);
}

#[derive(Clone, Copy, Default)]
pub(crate) struct CpuDiagnostic {
    pub task: usize,
    pub task_state: &'static str,
    pub preempt_depth: usize,
    pub migration_depth: usize,
    pub migration_pinned: usize,
    pub irq_mirror: u8,
    pub irq_mirror_at: u64,
    pub irq_depth: usize,
    pub in_trap_depth: usize,
    pub trap_imbalance: usize,
    pub switches: u64,
    pub last_switch: u64,
    pub need_resched: bool,
    pub rq_len: Option<usize>,
    pub loaded_mm: bool,
    pub loaded_asid: usize,
    pub mm_local_generation: u64,
}

/// §10 cross-CPU panic-path snapshot for the TLB shootdown timeout dump.
/// Racy reads of live CPUs are diagnostic-only: every shared field is an
/// atomic or an owned-CPU scalar that the panic dump may observe mid-update.
pub(crate) fn cpu_diagnostic(cpu: CpuId) -> CpuDiagnostic {
    let local = cpu_local(cpu);
    let mut diagnostic = CpuDiagnostic {
        irq_mirror: local.irq_mirror.load(Ordering::Relaxed),
        irq_mirror_at: local.irq_mirror_at.load(Ordering::Relaxed),
        irq_depth: local.irq_depth,
        in_trap_depth: local.in_trap_depth.load(Ordering::Relaxed),
        trap_imbalance: local.trap_imbalance.load(Ordering::Relaxed),
        switches: local.context_switches.load(Ordering::Relaxed),
        last_switch: local.last_switch.load(Ordering::Relaxed),
        need_resched: local.need_resched.load(Ordering::Relaxed),
        ..CpuDiagnostic::default()
    };

    if scheduler_ready() {
        let raw = local.current.load(Ordering::Acquire);
        if raw != NO_CPU {
            let current = TaskId::from_raw(raw);
            diagnostic.task = current.raw();
            let task = task_ref(current);
            diagnostic.task_state = match TaskState::decode(task.state.load(Ordering::Acquire)) {
                TaskState::Runnable => "runnable",
                TaskState::Running(_) => "running",
                TaskState::SwitchingOut(_) => "switching-out",
                TaskState::Blocked => "blocked",
                TaskState::Idle(_) => "idle",
                TaskState::Exited => "exited",
            };
            diagnostic.preempt_depth = task.preempt_disable_depth.load(Ordering::Acquire);
            diagnostic.migration_depth = task.migration_disable_depth.load(Ordering::Acquire);
            diagnostic.migration_pinned = task.migration_pinned.load(Ordering::Acquire);
        }

        // SAFETY: panic-path racy read of the loaded-mm slot. Only the
        // discriminant and the mm's atomic generation mirrors are consumed;
        // the Arc is never cloned or dereferenced for non-atomic state.
        let mm = unsafe { (&*local.loaded_mm.get()).as_ref() };
        if let Some(mm) = mm {
            diagnostic.loaded_mm = true;
            diagnostic.loaded_asid = usize::from(mm.asid().id().get());
            diagnostic.mm_local_generation = mm.local_tlb_generation(cpu);
        }

        if let Some(rq) = local.rq.try_lock() {
            diagnostic.rq_len = Some(rq.len());
        }
    }

    diagnostic
}

pub(crate) fn current_task_id() -> TaskId {
    let cpu = crate::smp::current_cpu_id();
    current_of(cpu)
}

/// Complete an in-flight switch from trap context when a trap lands after the
/// hardware stack switch but before the normal Rust switch-tail committed it.
///
/// Per-CPU identity needs no repair here: on RISC-V `tp` is per-CPU hardware
/// state that switch.S never touches and the user-trap anchor is rebuilt under
/// SIE-clear at every sret, and on LoongArch `current_cpu_id()` reads the
/// per-core CSR rather than the task-owned r21 register. A trap-top scan over
/// every CPU's `current`/`pending` is a hot-path violation of the A1.5
/// invariant and can only write stale cross-CPU data into a correct identity.
pub(crate) fn repair_current_cpu_from_stack() {
    if !scheduler_ready() {
        return;
    }
    let cpu = crate::smp::current_cpu_id();
    if cpu_local(cpu).pending().is_some() {
        // The trap landed after the hardware stack switch and before the
        // normal Rust switch-tail committed it. Complete that transaction
        // before the trap path can request another yield/preemption.
        finish_switch();
    }
}

pub(super) fn block_current_on_if<F>(queue: &WaitQueue, should_block: F) -> bool
where
    F: FnOnce() -> bool,
{
    crate::context::might_sleep();
    let interrupt_guard = crate::context::IrqSaveGuard::new();
    let cpu = crate::smp::current_cpu_id();
    let switch = {
        assert_schedulable(cpu);
        // The blocking decision must be atomic with waiter linkage: this
        // check and wake_waiters both run under SCHEDULER, so a completion
        // racing the check is either visible here or finds the linked waiter.
        // Closures must therefore not acquire locks whose holders wake
        // waiters while still holding them (all call sites are audited).
        let mut slot = SCHEDULER.lock();
        let scheduler = slot
            .as_mut()
            .expect("kernel scheduler is not initialized");
        if should_block() {
            Some(scheduler.prepare_block(cpu, queue))
        } else {
            None
        }
    };
    let Some((previous, next)) = switch else {
        drop(interrupt_guard);
        return false;
    };
    #[cfg(debug_assertions)]
    m4c_verify::before_block_context_switch(queue, cpu);
    // SAFETY: the outgoing task is held in SwitchingOut until the incoming
    // context completes the switch.
    unsafe { crate::arch::task::switch(previous, next) };
    finish_switch();
    drop(interrupt_guard);
    reap_retired_tasks();
    true
}

fn send_wakeup_ipis(targets: usize) {
    let current = crate::smp::current_cpu_id();
    for index in 0..crate::smp::discovered_cpu_count() {
        if targets & (1_usize << index) == 0 {
            continue;
        }
        let cpu = CpuId::new(index).expect("wakeup target exceeds MAX_CPUS");
        if cpu != current {
            crate::smp::send_ipi(cpu);
        }
    }
}

pub(super) fn wake_queue(queue: &WaitQueue, maximum: usize) -> usize {
    let (woken, targets) = {
        let mut slot = SCHEDULER.lock();
        let scheduler = slot.as_mut().expect("kernel scheduler is not initialized");
        scheduler.wake_waiters(queue, maximum, None)
    };
    send_wakeup_ipis(targets);
    woken
}

pub(super) fn wake_task_on_queue(queue: &WaitQueue, task: TaskId) -> bool {
    let (woken, targets) = {
        let mut slot = SCHEDULER.lock();
        let scheduler = slot.as_mut().expect("kernel scheduler is not initialized");
        scheduler.wake_waiters(queue, 1, Some(task))
    };
    send_wakeup_ipis(targets);
    assert!(woken <= 1, "targeted wake claimed multiple tasks");
    woken == 1
}

#[cfg(debug_assertions)]
pub(super) fn waiter_debug_state(channel: usize) -> WaiterDebugState {
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .waiter_debug_state(channel)
}

#[cfg(debug_assertions)]
pub(super) fn clear_current_affinity() {
    let cpu = crate::smp::current_cpu_id();
    let mut slot = SCHEDULER.lock();
    slot.as_mut()
        .expect("kernel scheduler is not initialized")
        .clear_current_affinity(cpu);
}

#[cfg(debug_assertions)]
pub(super) fn task_is_runnable_on(id: TaskId, cpu: CpuId) -> bool {
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .task_is_runnable_on(id, cpu)
}

#[cfg(debug_assertions)]
fn run_queue_len(cpu: CpuId) -> usize {
    // rq locks carry a rank above Timer, so they must never be held with
    // local interrupts enabled.
    let _irq_guard = crate::context::IrqSaveGuard::new();
    cpu_local(cpu).rq.lock().len()
}

#[cfg(debug_assertions)]
pub(super) fn migrate_runnable_task(id: TaskId, target: CpuId) {
    let source = {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .migrate_runnable_task(id, target)
    };

    assert_ne!(source, target);
    crate::smp::send_ipi(target);
}

pub fn register_secondary_cpu(cpu: CpuId) {
    assert!(crate::arch::interrupt::are_disabled());
    assert_eq!(
        cpu,
        crate::smp::current_cpu_id(),
        "CPU attempted to register another CPU's scheduler context",
    );

    {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .register_secondary(cpu);
    }
    crate::smp::mark_current_scheduler_registered();
}

fn mark_current_active() {
    assert!(
        crate::arch::interrupt::are_enabled(),
        "CPU became scheduler-active with local interrupts disabled",
    );
    let cpu = crate::smp::current_cpu_id();

    {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .activate_secondary(cpu);
    }
    crate::smp::mark_current_scheduler_active();
}

pub fn finalize_cpu_bringup() {
    assert_eq!(crate::smp::current_cpu_id(), CpuId::BOOT);
    assert!(crate::arch::interrupt::are_enabled());

    let registered_mask = registered_cpu_mask();

    crate::smp::assert_bringup_complete();

    let online_mask = crate::smp::online_cpu_mask();
    let active_mask = crate::smp::scheduler_active_cpu_mask();
    let ready_mask = crate::smp::ipi_ready_cpu_mask();

    assert_eq!(
        registered_mask, online_mask,
        "scheduler contexts and lifecycle-online CPUs diverged",
    );
    assert_eq!(
        active_mask, ready_mask,
        "scheduler-active and IPI-ready CPU masks diverged",
    );

    crate::println!("kernel scheduler CPUs:");
    crate::println!("  registered CPUs : {}", registered_mask.count_ones());
    crate::println!("  active CPUs     : {}", active_mask.count_ones());
    crate::println!("  active mask     : {:#x}", active_mask);
    crate::println!("  lifecycle source: smp::CpuState");
}

pub fn enter_secondary_idle() -> ! {
    assert!(crate::arch::interrupt::are_disabled());
    let cpu = crate::smp::current_cpu_id();
    assert_ne!(cpu, CpuId::BOOT);

    let next = {
        let slot = SCHEDULER.lock();
        slot.as_ref()
            .expect("kernel scheduler is not initialized")
            .secondary_idle_context(cpu)
    };
    let mut bootstrap = crate::arch::task::Context::default();

    // SAFETY: the secondary CPU owns its static bootstrap stack and has just
    // registered a distinct vmalloc-backed idle context. No task can run on
    // this CPU until the switch completes, and local interrupts are disabled.
    unsafe { crate::arch::task::switch(core::ptr::addr_of_mut!(bootstrap), next) };

    panic!("secondary bootstrap context resumed unexpectedly");
}

pub(crate) struct UserTaskHandle {
    id: TaskId,
    exit_visible: Arc<Completion>,
}

impl UserTaskHandle {
    pub(crate) const fn id(&self) -> TaskId {
        self.id
    }

    pub(crate) fn wait_for_exit_visible(&self) {
        EXIT_VISIBLE_WAIT_BEGIN.fetch_add(1, Ordering::Relaxed);
        self.exit_visible.wait();
        EXIT_VISIBLE_WAIT_END.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn release_process_owners(
        self,
        thread: Arc<crate::process::Thread>,
        process: Arc<crate::process::Process>,
    ) {
        assert_eq!(
            thread.process().id(),
            process.id(),
            "process-cleanup hand-off mixed Thread and Process owners",
        );
        // The retired Task independently retains the ownership needed for
        // safe deferred reclamation. The synchronous caller may release its
        // references immediately after ExitVisible.
        drop(thread);
        drop(process);
    }
}

pub(crate) fn spawn_user_thread_on(
    thread: Arc<crate::process::Thread>,
    affinity: Option<CpuId>,
) -> UserTaskHandle {
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();
    let stack = KernelStack::allocate()
        .unwrap_or_else(|error| panic!("unable to allocate user-thread kernel stack: {error:?}"));
    let exit_visible = Arc::new(Completion::new());
    let process_cleanup = ProcessCleanup {
        process: thread.process_arc(),
    };
    let (id, target) = {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .spawn_user(
                thread,
                Arc::clone(&exit_visible),
                Some(process_cleanup),
                stack,
                affinity,
                affinity,
            )
    };
    if target != crate::smp::current_cpu_id() {
        crate::smp::send_ipi(target);
    }
    UserTaskHandle { id, exit_visible }
}

pub(crate) fn spawn_user_thread_from_user_trap(
    thread: Arc<crate::process::Thread>,
) -> UserTaskHandle {
    let stack = KernelStack::allocate()
        .unwrap_or_else(|error| panic!("unable to allocate cloned user-thread stack: {error:?}"));
    let exit_visible = Arc::new(Completion::new());
    let _interrupt_guard = crate::context::IrqSaveGuard::new();
    let current = crate::smp::current_cpu_id();
    let (id, target) = {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .spawn_user(thread, Arc::clone(&exit_visible), None, stack, None, None)
    };
    if target != current {
        crate::smp::send_ipi(target);
    }
    UserTaskHandle { id, exit_visible }
}

pub(crate) fn current_user_thread() -> Option<Arc<crate::process::Thread>> {
    let cpu = crate::smp::current_cpu_id();
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .current_user_thread(cpu)
}

pub(crate) fn scheduler_is_initialized() -> bool {
    // smp-pcpu-v1: lockless gate.  TrackedSpinLock's debug path calls this
    // before taking any lock; an SCHEDULER acquisition here would recurse
    // whenever a rq lock is taken under REGISTRY.
    scheduler_ready()
}

pub(crate) fn user_mm_switches() -> u64 {
    mm_switches_total()
}

pub(crate) fn assert_user_mm_quiescent() {
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .assert_user_mm_quiescent();
}

pub(crate) fn replace_current_user_mm(
    old_mm: Arc<crate::user_mm::UserMm>,
    new_mm: Arc<crate::user_mm::UserMm>,
) {
    // smp-pcpu-v1: loaded_mm is truly-local CpuLocal state.  The IRQ-save
    // guard pins the CPU for the whole repair, satisfying the exclusive
    // quiescent-write contract of the accessors.
    let _interrupt_guard = crate::context::IrqSaveGuard::new();
    let cpu = crate::smp::current_cpu_id();
    assert!(
        task_ref(current_of(cpu)).user_thread.is_some(),
        "attempted to replace mm outside a user task",
    );
    let local = cpu_local(cpu);
    // Release/contest: repair loaded-mm tracking if it diverged.
    // The exec path may arrive with a stale or missing CPU loaded-mm
    // after a fatal fault or rapid task switches; panicking here kills
    // the entire contest run.
    let loaded_matches = local
        .loaded_mm()
        .as_ref()
        .map_or(false, |loaded| Arc::ptr_eq(loaded, &old_mm));
    if !loaded_matches {
        #[cfg(debug_assertions)]
        crate::println!(
            "scheduler: exec loaded-mm mismatch cpu={}; repairing",
            cpu.get(),
        );
        // Deactivate whatever is currently loaded (if anything).
        if let Some(stale) = local.take_loaded_mm() {
            let _ = stale.deactivate_current_cpu();
        }
    }
    if let Some(ref current_loaded) = local.loaded_mm() {
        if Arc::ptr_eq(current_loaded, &old_mm) {
            old_mm
                .deactivate_current_cpu()
                .unwrap_or_else(|error| crate::println!("exec: failed to leave old mm: {error:?}"));
        }
    }
    new_mm
        .activate_current_cpu()
        .unwrap_or_else(|error| crate::println!("exec: failed to enter new mm: {error:?}"));
    local.store_loaded_mm(Some(new_mm));
}

pub fn spawn_kernel_thread(entry: KernelThreadEntry) -> TaskId {
    spawn_internal(entry, None, None).0
}

pub(crate) fn spawn_kernel_thread_on(entry: KernelThreadEntry, cpu: CpuId) -> TaskId {
    spawn_internal(entry, Some(cpu), Some(cpu)).0
}

pub(crate) fn run_kernel_thread_sync(entry: KernelThreadEntry) {
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();
    assert_eq!(
        crate::smp::current_cpu_id(),
        CpuId::BOOT,
        "synchronous kernel-thread launcher must run on the boot CPU",
    );
    let caller_is_idle = {
        task_ref(current_of(CpuId::BOOT)).kind.is_idle()
    };
    assert!(
        caller_is_idle,
        "synchronous launcher must run from boot idle"
    );
    assert_eq!(
        counted_kernel_threads(),
        0,
        "synchronous launcher requires a quiescent counted thread set",
    );

    let (_, target) = spawn_internal(entry, Some(CpuId::BOOT), Some(CpuId::BOOT));
    assert_eq!(target, CpuId::BOOT);

    while counted_kernel_threads() != 0 {
        reap_retired_tasks();
        if current_cpu_has_work() {
            yield_now();
        } else {
            idle_until_interrupt();
        }
    }
    TASK_REAPER_QUEUE.wake_one();
}

#[cfg(debug_assertions)]
pub(crate) fn run_verifier_thread(entry: KernelThreadEntry) {
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();
    assert_eq!(
        crate::smp::current_cpu_id(),
        CpuId::BOOT,
        "timer verifier launcher must run on the boot CPU",
    );

    let caller_is_idle = {
        let current = current_of(CpuId::BOOT);
        task_ref(current).kind.is_idle()
    };
    assert!(
        caller_is_idle,
        "timer verifier launcher must run from the boot idle task",
    );
    assert_eq!(
        live_kernel_threads(),
        0,
        "timer verifier requires a quiescent counted kernel-thread set",
    );

    // M6-B r1: verifier launcher follows the real idle path.
    //
    // Keep the verifier on CPU0.  The boot idle task may then stop its local
    // scheduler tick and rely on the verifier's local one-shot deadline (or a
    // local runnable wakeup) to make progress.  An unpinned verifier could run
    // remotely while CPU0 has no local deadline and no completion IPI.
    let (_, target) = spawn_internal(entry, Some(CpuId::BOOT), Some(CpuId::BOOT));
    assert_eq!(target, CpuId::BOOT, "verifier was queued away from CPU0");

    let worker_deadline = verification_deadline();
    while live_kernel_threads() != 0 {
        assert!(
            !deadline_reached(crate::arch::time::counter(), worker_deadline),
            "timer verifier kernel thread timed out",
        );

        // This function executes on the boot idle task.  Reuse the same
        // decision as idle_loop(): schedule runnable work, otherwise perform
        // the IRQ-disabled final recheck and enter the architecture idle path.
        // Busy polling here would keep the scheduler tick active and would
        // make delayed-work/nohz verification impossible by construction.
        reap_retired_tasks();
        if current_cpu_has_work() {
            yield_now();
        } else {
            idle_until_interrupt();
        }
    }

    // complete_switch() publishes the retired-task lifetime while holding the
    // scheduler lock before live_kernel_threads() can be observed as zero.
    // Do not synchronously destroy the just-exited verifier stack on the boot
    // idle launcher's path: kernel-stack vfree performs global TLB work, and
    // Linux keeps that kind of stack lifetime handoff deferred to a reaper
    // context rather than folding it into the idle hand-off itself.
    TASK_REAPER_QUEUE.wake_one();
}

fn spawn_system_thread(
    entry: KernelThreadEntry,
    affinity: Option<CpuId>,
    queue_hint: Option<CpuId>,
) -> (TaskId, CpuId) {
    let stack = KernelStack::allocate()
        .unwrap_or_else(|error| panic!("unable to allocate system-thread stack: {error:?}"));

    let (id, target) = {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .spawn(
                entry,
                stack,
                affinity,
                queue_hint,
                true,
                TaskKind::SystemThread,
            )
    };

    if target != crate::smp::current_cpu_id() {
        crate::smp::send_ipi(target);
    }

    (id, target)
}

pub(crate) fn spawn_system_thread_on(entry: KernelThreadEntry, cpu: CpuId) -> TaskId {
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();
    assert!(
        crate::smp::is_scheduler_active(cpu),
        "system-thread target CPU is not scheduler-active",
    );
    let (id, target) = spawn_system_thread(entry, Some(cpu), Some(cpu));
    assert_eq!(
        target, cpu,
        "pinned system thread was queued on the wrong CPU"
    );
    id
}

fn spawn_internal(
    entry: KernelThreadEntry,
    affinity: Option<CpuId>,
    queue_hint: Option<CpuId>,
) -> (TaskId, CpuId) {
    let stack = KernelStack::allocate()
        .unwrap_or_else(|error| panic!("unable to allocate kernel-thread stack: {error:?}"));

    let (id, target) = {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .spawn(
                entry,
                stack,
                affinity,
                queue_hint,
                true,
                TaskKind::KernelThread,
            )
    };

    if target != crate::smp::current_cpu_id() {
        crate::smp::send_ipi(target);
    }

    (id, target)
}

#[cfg(debug_assertions)]
fn spawn_queued_without_reschedule(
    entry: KernelThreadEntry,
    affinity: Option<CpuId>,
    queue_hint: Option<CpuId>,
) -> (TaskId, CpuId) {
    let stack = KernelStack::allocate()
        .unwrap_or_else(|error| panic!("unable to allocate kernel-thread stack: {error:?}"));

    let mut slot = SCHEDULER.lock();
    slot.as_mut()
        .expect("kernel scheduler is not initialized")
        .spawn(
            entry,
            stack,
            affinity,
            queue_hint,
            false,
            TaskKind::KernelThread,
        )
}

pub fn yield_now() {
    crate::context::assert_interrupts_enabled();

    let interrupt_guard = crate::context::IrqSaveGuard::new();
    let cpu = crate::smp::current_cpu_id();
    // smp-pcpu-v1: the Yield fast path never touches the REGISTRY.  Local rq
    // locks carry a rank above Timer, so the switch runs with IRQs off via
    // the guard above and the atomic per-CPU transaction below.
    assert_schedulable(cpu);
    let switch = prepare_yield_local(cpu);

    let Some((previous, next)) = switch else {
        return;
    };

    // SAFETY: the old task is marked SwitchingOut and cannot be selected by
    // another CPU. The incoming task is exclusively Running on this CPU, both
    // contexts remain allocated, and local interrupts stay disabled.
    unsafe { crate::arch::task::switch(previous, next) };

    finish_switch();
    drop(interrupt_guard);
    reap_retired_tasks();
}

pub(crate) fn yield_from_user_trap() {
    let interrupt_guard = crate::context::IrqSaveGuard::new();
    let cpu = crate::smp::current_cpu_id();
    assert_schedulable(cpu);
    let switch = prepare_yield_local(cpu);
    let Some((previous, next)) = switch else {
        return;
    };

    // SAFETY: this is the synchronous syscall trap path with local interrupts
    // disabled. Both task contexts and their kernel stacks remain scheduler-owned.
    unsafe { crate::arch::task::switch(previous, next) };
    finish_switch();
    drop(interrupt_guard);
}

pub(crate) fn block_current_on_if_from_user_trap<F>(queue: &WaitQueue, should_block: F) -> bool
where
    F: FnOnce() -> bool,
{
    let interrupt_guard = crate::context::IrqSaveGuard::new();
    let cpu = crate::smp::current_cpu_id();
    let switch = {
        assert_schedulable(cpu);
        // Same check-link atomicity as block_current_on_if: the predicate is
        // re-evaluated under SCHEDULER so a racing completion either satisfies
        // it here or wakes the waiter after it is linked.
        // SUDOOS_FORCED_EXIT_PIPE_BREAK_V25: exit_group / execve sibling
        // teardown sets forced_exit_status and wakes only tasks that are
        // already blocked at scan time. A thread that was still running
        // during the scan must observe the flag here, under the same
        // SCHEDULER lock the flag write uses, or it blocks after the scan
        // finished and nothing ever wakes it again (lost wakeup). Callers
        // that loop must therefore re-check the flag after a false return.
        // current_user_thread() takes SCHEDULER itself, so fetch the Arc
        // here; the flag load below runs under SCHEDULER and is therefore
        // atomic with the teardown scan's flag write (same lock).
        let user_thread = current_user_thread();
        let mut slot = SCHEDULER.lock();
        let scheduler = slot
            .as_mut()
            .expect("kernel scheduler is not initialized");
        let forced_exit = user_thread
            .as_ref()
            .and_then(|thread| thread.forced_exit_status())
            .is_some();
        if !forced_exit && should_block() {
            Some(scheduler.prepare_block(cpu, queue))
        } else {
            None
        }
    };
    let Some((previous, next)) = switch else {
        drop(interrupt_guard);
        return false;
    };
    // SAFETY: this mirrors sched_yield from the user trap path. The current
    // task is linked into the wait queue before the switch and can be woken by
    // another task before this syscall resumes.
    unsafe { crate::arch::task::switch(previous, next) };
    finish_switch();
    drop(interrupt_guard);
    true
}

fn preempt_schedule() {
    crate::context::assert_interrupts_enabled();
    let interrupt_guard = crate::context::IrqSaveGuard::new();
    preempt_schedule_disabled();
    drop(interrupt_guard);
    reap_retired_tasks();
}

fn preempt_schedule_irq() {
    crate::context::assert_interrupts_disabled();
    preempt_schedule_disabled();
}

fn preempt_schedule_disabled() {
    if !scheduler_ready() {
        return;
    }
    let cpu = crate::smp::current_cpu_id();
    // smp-pcpu-v1: the Preempt fast path never touches the REGISTRY.
    let switch = prepare_preempt_local(cpu);

    let Some((previous, next)) = switch else {
        return;
    };

    // SAFETY: the timer/IPI exit path has dropped IRQ depth to zero and the
    // scheduler has exclusively assigned the incoming context to this CPU.
    unsafe { crate::arch::task::switch(previous, next) };
    finish_switch();
}

fn exit_current() -> ! {
    crate::context::assert_interrupts_enabled();

    let _interrupt_guard = crate::context::IrqSaveGuard::new();
    // Per-CPU identity is trustworthy without any stack scan: RISC-V `tp` is
    // per-CPU hardware state (switch.S never touches it, and the user-trap
    // anchor is rebuilt from the register under SIE-clear at sret), and
    // LoongArch reads the per-core identity CSR. A scan over every CPU's
    // `current` would be a hot-path A1.5 violation and could only overwrite a
    // correct identity with stale cross-CPU data.
    let actual_cpu = crate::smp::current_cpu_id();
    crate::arch::smp::set_current_cpu_id(actual_cpu.get());
    let (previous, next) = {
        assert_schedulable(actual_cpu);
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .prepare_exit(actual_cpu)
    };
    // SAFETY: the exiting task remains allocated and marked SwitchingOut
    // until the incoming context calls finish_switch() from a different stack.
    unsafe { crate::arch::task::switch(previous, next) };

    panic!("exited kernel thread resumed unexpectedly");
}

fn finish_switch() {
    let running_sp = crate::arch::task::current_stack_pointer();
    // The switch tail runs on the CPU that performed the hardware switch.
    // Its identity is trustworthy without any stack scan: RISC-V `tp` is
    // per-CPU hardware state that switch.S never touches (the user-trap
    // anchor is rebuilt from the register under SIE-clear at sret), and
    // LoongArch reads the per-core identity CSR. A scan over every CPU's
    // current/pending would violate the A1.5 hot-path invariant and could
    // only corrupt a correct identity with stale cross-CPU data.
    let cpu = crate::smp::current_cpu_id();
    crate::arch::smp::set_current_cpu_id(cpu.get());
    // smp-pcpu-v1 dispatch: Yield commits locklessly with only local rq and
    // task atomics; Block/Exit mutate registry-owned task state and take the
    // global REGISTRY for the commit.
    // The secondary's first switch into its idle context has no recorded
    // per-CPU transaction: register_secondary already committed
    // current/state under REGISTRY before the bootstrap switch. After a CPU
    // activates, every switch tail must carry a recorded transaction.
    let Some(pending) = cpu_local(cpu).pending() else {
        assert!(
            !crate::smp::is_scheduler_active(cpu),
            "switch tail without a pending switch"
        );
        return;
    };
    let (completed, current_is_idle) = match pending.disposition {
        SwitchDisposition::Yield => {
            let is_idle = complete_yield_switch(cpu, pending, running_sp);
            (
                CompletedSwitch {
                    retired_task_added: false,
                    exit_visible: None,
                },
                is_idle,
            )
        }
        SwitchDisposition::Block | SwitchDisposition::Exit => {
            let mut slot = SCHEDULER.lock();
            slot.as_mut()
                .expect("kernel scheduler is not initialized")
                .complete_registry_switch(cpu, pending, running_sp)
        }
    };
    // The hardware switch has committed; retire the per-CPU transaction so
    // the next local prepare can proceed.
    cpu_local(cpu).store_pending(None);

    // The switch tail still owns the IRQ-save guard, but no longer owns the
    // registry lock. Restore policy ticks here to preserve Timer < Scheduler.
    if !current_is_idle {
        crate::time::leave_idle();
    }
    if let Some(exit_visible) = completed.exit_visible {
        // The hardware switch has committed and the registry lock is no
        // longer held. The retired Task deliberately retains Thread/Process
        // ownership until the reaper has destroyed the old kernel stack.
        USER_TASKS_EXIT_VISIBLE.fetch_add(1, Ordering::Release);
        exit_visible.completion.complete_all();
    }
    if completed.retired_task_added {
        TASK_REAPER_QUEUE.wake_one();
    }
}

fn drain_retired_queue() {
    loop {
        // Phase 1: detach one dead task while holding only the scheduler lock.
        let retired = {
            let mut slot = SCHEDULER.lock();
            slot.as_mut()
                .expect("kernel scheduler is not initialized")
                .take_retired_task()
        };
        let Some(task) = retired else {
            break;
        };

        // Phase 2: release VM mappings, page-table pages and the guarded stack
        // with no scheduler/cross-CPU spin lock held.  Timer IRQs and TLB/IPI
        // completion paths are therefore free to make forward progress.
        task.destroy_resources();
        TASKS_RECLAIMED.fetch_add(1, Ordering::Relaxed);
        complete_retired_task_reclamation();
    }
}

fn complete_retired_task_reclamation() {
    // Release publishes all destruction side effects before a waiter observes
    // the final zero through its Acquire load.
    let outstanding = RETIRED_OUTSTANDING
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_sub(1)
        })
        .expect("retired outstanding counter underflowed");
    if outstanding == 1 {
        TASK_REAPER_DRAINED.wake_all();
    }
}

fn task_reaper_main() {
    loop {
        TASK_REAPER_QUEUE.wait_until(|| retired_task_backlog() != 0);
        drain_retired_queue();
    }
}

fn reap_retired_tasks() {
    if retired_task_backlog() != 0 {
        TASK_REAPER_QUEUE.wake_one();
    }
}

#[cfg(debug_assertions)]
pub(super) fn synchronize_retired_tasks_with_live(expected_live: usize) {
    crate::context::might_sleep();
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();
    assert_eq!(
        live_kernel_threads(),
        expected_live,
        "task reclamation barrier observed the wrong live verifier count",
    );

    let deadline = verification_deadline();

    while retired_task_outstanding() != 0 {
        // This debug barrier is itself task context with interrupts enabled.
        // Drain directly so verifier progress does not depend on the permanent
        // reaper being scheduled between two idle/verifier hand-offs, and so a
        // spurious reaper wake cannot run before the boot idle launcher regains
        // control after a nested verifier thread exits.
        drain_retired_queue();
        if retired_task_outstanding() == 0 {
            break;
        }

        assert!(
            !deadline_reached(crate::arch::time::counter(), deadline),
            "retired task reclamation timed out: backlog={} outstanding={} live={}",
            retired_task_backlog(),
            retired_task_outstanding(),
            live_kernel_threads(),
        );
        assert_eq!(
            live_kernel_threads(),
            expected_live,
            "counted verifier population changed during reclamation",
        );
        yield_now();
        spin_loop();
    }

    if current_cpu_has_work() {
        // A retiring task wakes the permanent reaper from the switch tail. This
        // verifier barrier may have already drained the queue directly; let the
        // reaper consume that now-empty wake and block again before a nested
        // verifier exits back into the boot idle launcher.
        yield_now();
    }

    assert_eq!(
        retired_task_backlog(),
        0,
        "retired task backlog remained after reclamation barrier",
    );
    let retired = retired_task_count();
    assert_eq!(retired, 0, "retired task queue was not fully drained");
}

#[cfg(debug_assertions)]
fn synchronize_retired_tasks() {
    synchronize_retired_tasks_with_live(0);
}

fn current_entry() -> KernelThreadEntry {
    let cpu = crate::smp::current_cpu_id();
    task_ref(current_of(cpu))
        .entry
        .expect("current task has no kernel-thread entry")
}

fn current_cpu_has_work() -> bool {
    let cpu = crate::smp::current_cpu_id();
    work_available(cpu)
}

#[cfg(debug_assertions)]
fn current_stack_contains(address: usize) -> bool {
    let cpu = crate::smp::current_cpu_id();
    task_ref(current_of(cpu)).stack_contains(address)
}

#[cfg(debug_assertions)]
fn active_cpu_count() -> usize {
    crate::smp::scheduler_active_cpu_count()
}

#[cfg(debug_assertions)]
fn active_cpu_mask() -> usize {
    crate::smp::scheduler_active_cpu_mask()
}

// This counter is part of the runtime reaper/idle protocol, not a verifier-only
// diagnostic. Keep it available in release builds as well.
fn retired_task_backlog() -> usize {
    RETIRED_BACKLOG.load(Ordering::Acquire)
}

fn retired_task_outstanding() -> usize {
    RETIRED_OUTSTANDING.load(Ordering::Acquire)
}

pub(crate) fn synchronize_user_task_reclamation() {
    crate::context::might_sleep();
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();

    // The public OS competition runners are launched synchronously from the
    // boot idle task.  A just-exited verifier can still be queued for deferred
    // reclamation when control returns to that idle context, and idle tasks
    // must never join a WaitQueue.  Drain queued work directly first; if a
    // reaper on another CPU owns the last item, idle with interrupts enabled
    // until its completion wake becomes observable.
    let caller_is_idle = {
        let cpu = crate::smp::current_cpu_id();
        task_ref(current_of(cpu)).kind.is_idle()
    };
    if caller_is_idle {
        while retired_task_outstanding() != 0 {
            drain_retired_queue();
            if retired_task_outstanding() == 0 {
                break;
            }
            TASK_REAPER_QUEUE.wake_one();
            if current_cpu_has_work() {
                yield_now();
            } else {
                idle_until_interrupt();
            }
        }
    } else {
        if retired_task_outstanding() != 0 {
            TASK_REAPER_QUEUE.wake_one();
        }
        TASK_REAPER_DRAINED.wait_until(|| retired_task_outstanding() == 0);
    }
}

#[cfg(debug_assertions)]
fn retired_task_count() -> usize {
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .retired_task_count()
}

fn counted_kernel_threads() -> usize {
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .live_kernel_threads
}

#[cfg(debug_assertions)]
fn live_kernel_threads() -> usize {
    counted_kernel_threads()
}

#[cfg(debug_assertions)]
fn context_switches() -> u64 {
    context_switches_total()
}

#[cfg(debug_assertions)]
pub(super) fn preemptions() -> u64 {
    preemptions_total()
}

unsafe extern "C" fn user_thread_bootstrap() -> ! {
    finish_switch();

    // SAFETY: the task owns a guarded kernel stack, the scheduler has installed
    // its UserMm for this CPU, and trap/IPI/timer state is fully initialized.
    unsafe { crate::arch::interrupt::enable() };

    let thread = current_user_thread().expect("M9-B user task lost its Thread binding");
    thread
        .mark_running()
        .expect("M9-B user Thread entered an invalid lifecycle state");
    // A clone may have joined the Process just before exit_group/exec closes
    // admission, then become runnable only after the sibling-exit scan. The
    // scheduler's late check marks it for exit; honor that mark before ever
    // entering the obsolete user context.
    let result = if let Some(status) = thread.forced_exit_status() {
        // clone saves the initial user trap frame before publishing the task.
        // If exec/exit_group wins before the task's first dispatch, that frame
        // will never be consumed by run_scheduled_thread; discard it before
        // Thread::drop validates ownership.
        let _ = thread.take_trap_frame();
        status
    } else {
        crate::user::run_scheduled_thread(&thread)
    };

    // User return paths intentionally restore a kernel-mode trap frame, and
    // some architectures leave local interrupts disabled at that boundary.
    // Thread teardown and task exit are ordinary sleepable task-context work.
    if crate::arch::interrupt::are_disabled() {
        // SAFETY: we are back on this task's kernel stack with no scheduler
        // lock held; the user trap frame has already been consumed.
        unsafe { crate::arch::interrupt::enable() };
    }
    crate::user::cleanup_robust_list_on_exit(&thread);
    if crate::user::oscomp_lifecycle_trace_active() {
        crate::println!("process-cleanup: tid={} robust done", thread.id().get());
    }
    crate::user::clear_child_tid_on_exit(&thread);
    if crate::user::oscomp_lifecycle_trace_active() {
        crate::println!("process-cleanup: tid={} ctid done", thread.id().get());
    }
    thread
        .exit(result)
        .expect("M9-B user Thread failed to publish exit state");

    // `exit_current()` switches away without unwinding this bootstrap frame.
    // Release the scheduler lookup clone now; otherwise it remains stranded on
    // the retired kernel stack and keeps Thread -> Process alive after reaping.
    drop(thread);
    exit_current()
}

unsafe extern "C" fn kernel_thread_bootstrap() -> ! {
    finish_switch();

    // SAFETY: trap entry, the per-CPU timer/IPI sources, and this task's
    // guarded kernel stack are installed before a fresh thread is selected.
    unsafe { crate::arch::interrupt::enable() };

    let entry = current_entry();
    entry();
    exit_current()
}

#[unsafe(no_mangle)]
pub extern "C" fn kernel_fresh_context_returned() -> ! {
    panic!("fresh task bootstrap unexpectedly returned")
}

unsafe extern "C" fn idle_thread_bootstrap() -> ! {
    finish_switch();

    // SAFETY: secondary initialization installed its trap vector, local timer,
    // IPI source and permanent guarded idle stack before entering this context.
    unsafe { crate::arch::interrupt::enable() };
    note_irq_state(true);

    // Publish scheduler eligibility before IPI readiness. The boot CPU waits
    // for the IPI-ready mask before leaving bring-up, so an active secondary
    // cannot be targeted by normal work until it can also receive the kick.
    mark_current_active();
    crate::smp::mark_current_ipi_ready();

    reap_retired_tasks();
    idle_loop()
}

fn idle_loop() -> ! {
    loop {
        reap_retired_tasks();
        if current_cpu_has_work() {
            yield_now();
        } else {
            idle_until_interrupt();
        }
    }
}

fn idle_until_interrupt() {
    crate::arch::interrupt::disable();
    note_irq_state(false);

    if current_cpu_has_work() || retired_task_backlog() != 0 {
        // SAFETY: this CPU is already in a fully initialized idle context; the
        // caller will immediately leave the idle path and schedule/reap work.
        unsafe { crate::arch::interrupt::enable() };
        note_irq_state(true);
        IDLE_EXITS[crate::smp::current_cpu_id().get()].fetch_add(1, Ordering::AcqRel);
        return;
    }

    let cpu = crate::smp::current_cpu_id();
    // Keep every CPU's scheduler clockevent active while it is idle.  Runnable
    // user tasks become CPU-affine after their first dispatch, so another CPU
    // cannot steal a woken task if the target's reschedule IPI is coalesced at
    // the WFI boundary.  The periodic tick is therefore the level-triggered
    // fallback which turns an already-published `need_resched` bit into a
    // context switch instead of leaving that CPU asleep indefinitely.
    #[cfg(debug_assertions)]
    idle_verify::before_arch_wait(cpu);

    IDLE_ENTERS[cpu.get()].fetch_add(1, Ordering::AcqRel);

    // SAFETY: the idle task has a valid trap frame, local interrupt sources are
    // configured, and work was rechecked with local interrupts disabled.
    unsafe { crate::arch::cpu::enable_and_wait_for_interrupt() };
    note_irq_state(true);

    IDLE_EXITS[cpu.get()].fetch_add(1, Ordering::AcqRel);
}

#[cfg(debug_assertions)]
fn idle_counter_totals() -> (u64, u64) {
    let enters = IDLE_ENTERS
        .iter()
        .map(|counter| counter.load(Ordering::Acquire))
        .sum();
    let exits = IDLE_EXITS
        .iter()
        .map(|counter| counter.load(Ordering::Acquire))
        .sum();

    (enters, exits)
}

pub fn boot_idle_loop() -> ! {
    assert_eq!(crate::smp::current_cpu_id(), CpuId::BOOT);
    idle_loop()
}

#[cfg(debug_assertions)]
static WORKER_PROGRESS: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];
#[cfg(debug_assertions)]
static WORKER_STACKS: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];
#[cfg(debug_assertions)]
static WORKER_CPUS: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(usize::MAX) }; MAX_CPUS];
#[cfg(debug_assertions)]
static EXPECTED_CPUS: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(usize::MAX) }; MAX_CPUS];
#[cfg(debug_assertions)]
static WORKER_READY_MASK: AtomicUsize = AtomicUsize::new(0);
#[cfg(debug_assertions)]
static EXPECTED_WORKER_MASK: AtomicUsize = AtomicUsize::new(0);
#[cfg(debug_assertions)]
static COMPLETED_WORKERS: AtomicUsize = AtomicUsize::new(0);
#[cfg(debug_assertions)]
static VERIFY_ITERATIONS: AtomicUsize = AtomicUsize::new(0);
#[cfg(debug_assertions)]
static USE_CONCURRENT_BARRIER: AtomicBool = AtomicBool::new(false);
#[cfg(debug_assertions)]
static STEAL_COMPLETED: AtomicUsize = AtomicUsize::new(0);
#[cfg(debug_assertions)]
static STEAL_CPU_MASK: AtomicUsize = AtomicUsize::new(0);

#[cfg(debug_assertions)]
fn verification_worker(index: usize) {
    let expected_cpu = CpuId::new(EXPECTED_CPUS[index].load(Ordering::Acquire))
        .expect("verification worker has no expected CPU");
    let cpu = crate::smp::current_cpu_id();
    assert_eq!(cpu, expected_cpu, "pinned worker ran on the wrong CPU");

    let canary = 0x1357_2468_aaaa_5555_usize ^ index.wrapping_mul(0x0101_0101_0101_0101);
    let address = core::ptr::addr_of!(canary) as usize;
    assert!(current_stack_contains(address));

    WORKER_STACKS[index].store(address, Ordering::Release);
    WORKER_CPUS[index].store(cpu.get(), Ordering::Release);
    WORKER_READY_MASK.fetch_or(1_usize << index, Ordering::AcqRel);

    if USE_CONCURRENT_BARRIER.load(Ordering::Acquire) {
        let deadline = verification_deadline();
        let expected = EXPECTED_WORKER_MASK.load(Ordering::Acquire);

        while WORKER_READY_MASK.load(Ordering::Acquire) & expected != expected {
            let ready = WORKER_READY_MASK.load(Ordering::Acquire);
            assert!(
                !deadline_reached(crate::arch::time::counter(), deadline),
                "SMP workers failed to execute concurrently: cpu={} ready={ready:#x} expected={expected:#x}",
                crate::smp::current_cpu_id().get(),
            );
            spin_loop();
        }
    }

    let iterations = VERIFY_ITERATIONS.load(Ordering::Acquire);
    for iteration in 0..iterations {
        assert_eq!(
            black_box(canary),
            0x1357_2468_aaaa_5555_usize ^ index.wrapping_mul(0x0101_0101_0101_0101)
        );
        WORKER_PROGRESS[index].store(iteration + 1, Ordering::Release);
        yield_now();
    }

    let ticks_before = crate::time::timer_ticks_for(cpu);
    while crate::time::timer_ticks_for(cpu) == ticks_before {
        crate::arch::cpu::wait_for_interrupt();
    }

    COMPLETED_WORKERS.fetch_add(1, Ordering::AcqRel);
}

#[cfg(debug_assertions)]
fn worker_0() {
    verification_worker(0);
}
#[cfg(debug_assertions)]
fn worker_1() {
    verification_worker(1);
}
#[cfg(debug_assertions)]
fn worker_2() {
    verification_worker(2);
}
#[cfg(debug_assertions)]
fn worker_3() {
    verification_worker(3);
}
#[cfg(debug_assertions)]
fn worker_4() {
    verification_worker(4);
}
#[cfg(debug_assertions)]
fn worker_5() {
    verification_worker(5);
}
#[cfg(debug_assertions)]
fn worker_6() {
    verification_worker(6);
}
#[cfg(debug_assertions)]
fn worker_7() {
    verification_worker(7);
}
#[cfg(all(debug_assertions, target_arch = "loongarch64"))]
fn worker_8() {
    verification_worker(8);
}
#[cfg(all(debug_assertions, target_arch = "loongarch64"))]
fn worker_9() {
    verification_worker(9);
}
#[cfg(all(debug_assertions, target_arch = "loongarch64"))]
fn worker_10() {
    verification_worker(10);
}
#[cfg(all(debug_assertions, target_arch = "loongarch64"))]
fn worker_11() {
    verification_worker(11);
}
#[cfg(all(debug_assertions, target_arch = "loongarch64"))]
fn worker_12() {
    verification_worker(12);
}
#[cfg(all(debug_assertions, target_arch = "loongarch64"))]
fn worker_13() {
    verification_worker(13);
}
#[cfg(all(debug_assertions, target_arch = "loongarch64"))]
fn worker_14() {
    verification_worker(14);
}
#[cfg(all(debug_assertions, target_arch = "loongarch64"))]
fn worker_15() {
    verification_worker(15);
}

#[cfg(all(debug_assertions, target_arch = "riscv64"))]
const WORKER_ENTRIES: [KernelThreadEntry; MAX_CPUS] = [
    worker_0, worker_1, worker_2, worker_3, worker_4, worker_5, worker_6, worker_7,
];
#[cfg(all(debug_assertions, target_arch = "loongarch64"))]
const WORKER_ENTRIES: [KernelThreadEntry; MAX_CPUS] = [
    worker_0, worker_1, worker_2, worker_3, worker_4, worker_5, worker_6, worker_7,
    worker_8, worker_9, worker_10, worker_11, worker_12, worker_13, worker_14, worker_15,
];

#[cfg(debug_assertions)]
fn steal_worker() {
    let cpu = crate::smp::current_cpu_id();
    assert_ne!(cpu, CpuId::BOOT, "boot CPU consumed a work-stealing task");
    STEAL_CPU_MASK.fetch_or(1_usize << cpu.get(), Ordering::AcqRel);
    STEAL_COMPLETED.fetch_add(1, Ordering::AcqRel);
}

#[cfg(debug_assertions)]
fn reset_verification_state() {
    for index in 0..MAX_CPUS {
        WORKER_PROGRESS[index].store(0, Ordering::Release);
        WORKER_STACKS[index].store(0, Ordering::Release);
        WORKER_CPUS[index].store(usize::MAX, Ordering::Release);
        EXPECTED_CPUS[index].store(usize::MAX, Ordering::Release);
    }
    WORKER_READY_MASK.store(0, Ordering::Release);
    EXPECTED_WORKER_MASK.store(0, Ordering::Release);
    COMPLETED_WORKERS.store(0, Ordering::Release);
    STEAL_COMPLETED.store(0, Ordering::Release);
    STEAL_CPU_MASK.store(0, Ordering::Release);
}

#[cfg(debug_assertions)]
fn verification_deadline() -> u64 {
    crate::arch::time::counter().wrapping_add(
        crate::time::clock_frequency_hz()
            .checked_mul(VERIFY_TIMEOUT_SECONDS)
            .expect("scheduler verification timeout overflowed"),
    )
}

#[cfg(debug_assertions)]
fn deadline_reached(now: u64, deadline: u64) -> bool {
    now.wrapping_sub(deadline) < (1_u64 << 63)
}

#[cfg(debug_assertions)]
fn wait_for_workers(worker_count: usize) {
    let deadline = verification_deadline();

    while COMPLETED_WORKERS.load(Ordering::Acquire) != worker_count || live_kernel_threads() != 0 {
        assert!(
            !deadline_reached(crate::arch::time::counter(), deadline),
            "kernel scheduler worker test timed out",
        );
        // Remote completion counters are plain atomic publications, not wake
        // events. Keep this debug verifier runnable instead of relying on a
        // periodic timer to escape WFI.  yield_now() completes its own switch
        // tail, so no pending transaction may remain once this loop exits.
        yield_now();
        spin_loop();
    }
}

#[cfg(debug_assertions)]
fn verify_ipi_delivery(cpu_count: usize) {
    if cpu_count == 1 {
        return;
    }

    let mut before = [0_u64; MAX_CPUS];
    for (index, value) in before.iter_mut().enumerate().take(cpu_count).skip(1) {
        let cpu = CpuId::new(index).expect("invalid CPU in IPI test");
        *value = crate::smp::ipi_count(cpu);
    }

    crate::smp::broadcast_ipi_except_current();
    let deadline = verification_deadline();

    loop {
        let delivered = before
            .iter()
            .enumerate()
            .take(cpu_count)
            .skip(1)
            .all(|(index, old)| {
                let cpu = CpuId::new(index).expect("invalid CPU in IPI test");
                crate::smp::ipi_count(cpu) > *old
            });

        if delivered {
            return;
        }

        assert!(
            !deadline_reached(crate::arch::time::counter(), deadline),
            "IPI delivery verification timed out",
        );
        // Remote CPUs only publish counters; they do not signal CPU0 back.
        spin_loop();
    }
}

#[cfg(debug_assertions)]
fn verify_work_stealing(cpu_count: usize) {
    if cpu_count == 1 {
        return;
    }

    STEAL_COMPLETED.store(0, Ordering::Release);
    STEAL_CPU_MASK.store(0, Ordering::Release);

    let preempt_guard = PreemptGuard::new();
    for _ in 0..STEAL_TASK_COUNT {
        spawn_queued_without_reschedule(steal_worker, None, Some(CpuId::BOOT));
    }

    crate::smp::broadcast_ipi_except_current();
    let deadline = verification_deadline();

    while STEAL_COMPLETED.load(Ordering::Acquire) != STEAL_TASK_COUNT || live_kernel_threads() != 0
    {
        assert!(
            !deadline_reached(crate::arch::time::counter(), deadline),
            "work-stealing verification timed out: completed={} live={} cpu0_queue={}",
            STEAL_COMPLETED.load(Ordering::Acquire),
            live_kernel_threads(),
            run_queue_len(CpuId::BOOT),
        );
        // Deliberately do not yield CPU0: runnable tasks queued on CPU0 must
        // be stolen and executed by secondary CPUs. A single publication kick
        // must be sufficient; repeated rescue IPIs would hide a lost-wakeup
        // defect in the scheduler/IPI path.
        spin_loop();
    }
    drop(preempt_guard);

    assert_ne!(
        STEAL_CPU_MASK.load(Ordering::Acquire) & !1_usize,
        0,
        "no secondary CPU stole a runnable task",
    );
    synchronize_retired_tasks();
}

// M7_COOPERATIVE_SWITCH_VERIFIER_V3_EXACT
// Validate one completed verifier phase before its shared slots are reused.
#[cfg(debug_assertions)]
fn verify_worker_results(worker_count: usize, iterations: usize) {
    for index in 0..worker_count {
        assert_eq!(WORKER_PROGRESS[index].load(Ordering::Acquire), iterations);
        assert_ne!(WORKER_STACKS[index].load(Ordering::Acquire), 0);
        assert_eq!(
            WORKER_CPUS[index].load(Ordering::Acquire),
            EXPECTED_CPUS[index].load(Ordering::Acquire),
        );
        for stack_slot in WORKER_STACKS.iter().take(index) {
            assert_ne!(
                WORKER_STACKS[index].load(Ordering::Acquire),
                stack_slot.load(Ordering::Acquire),
                "two kernel threads shared a stack",
            );
        }
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    reset_verification_state();
    crate::heap::shrink();

    let cpu_count = active_cpu_count();
    assert_eq!(cpu_count, crate::smp::discovered_cpu_count());
    assert_eq!(cpu_count, crate::smp::online_cpu_count());
    assert_eq!(
        active_cpu_mask(),
        crate::smp::ipi_ready_cpu_mask(),
        "scheduler-active and IPI-ready masks diverged before verification",
    );

    // Phase 1 proves topology, affinity, guarded private stacks, concurrent
    // execution on SMP, and timer coexistence. It does not pretend that a
    // lone runnable task's yield attempt must commit a task-to-task switch.
    let topology_worker_count = if cpu_count == 1 { 2 } else { cpu_count };
    let topology_iterations = if cpu_count == 1 {
        SINGLE_CPU_VERIFY_ITERATIONS
    } else {
        SMP_VERIFY_ITERATIONS
    };
    VERIFY_ITERATIONS.store(topology_iterations, Ordering::Release);
    USE_CONCURRENT_BARRIER.store(cpu_count > 1, Ordering::Release);
    EXPECTED_WORKER_MASK.store((1_usize << topology_worker_count) - 1, Ordering::Release);

    let pages_before = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable before scheduler verification");
    let switches_before = context_switches();

    let preempt_guard = PreemptGuard::new();
    for index in 0..topology_worker_count {
        let cpu = if cpu_count == 1 {
            CpuId::BOOT
        } else {
            CpuId::new(index).expect("worker CPU exceeds MAX_CPUS")
        };
        EXPECTED_CPUS[index].store(cpu.get(), Ordering::Release);
        if cpu_count == 1 {
            spawn_kernel_thread(WORKER_ENTRIES[index]);
        } else {
            spawn_queued_without_reschedule(WORKER_ENTRIES[index], Some(cpu), Some(cpu));
        }
    }
    for index in 0..topology_worker_count {
        let cpu = if cpu_count == 1 {
            CpuId::BOOT
        } else {
            CpuId::new(index).expect("worker CPU exceeds MAX_CPUS")
        };
        request_reschedule_on(cpu);
    }
    drop(preempt_guard);

    wait_for_workers(topology_worker_count);
    verify_worker_results(topology_worker_count, topology_iterations);

    // Phase 2 supplies the missing invariant on SMP: two peers are queued on
    // the same CPU before either can run. Every cooperative yield therefore
    // has a runnable peer and measures committed scheduler transitions rather
    // than calls to yield_now(). The single-CPU topology phase already has two
    // peers on CPU0 and is used directly.
    let cooperative_switches = if cpu_count == 1 {
        context_switches()
            .checked_sub(switches_before)
            .expect("context switch counter moved backwards")
    } else {
        synchronize_retired_tasks();
        reset_verification_state();
        VERIFY_ITERATIONS.store(COOPERATIVE_VERIFY_ITERATIONS, Ordering::Release);
        USE_CONCURRENT_BARRIER.store(false, Ordering::Release);
        EXPECTED_WORKER_MASK.store(0b11, Ordering::Release);

        let cooperative_before = context_switches();
        let preempt_guard = PreemptGuard::new();
        for index in 0..2 {
            EXPECTED_CPUS[index].store(CpuId::BOOT.get(), Ordering::Release);
            let (_, target) = spawn_queued_without_reschedule(
                WORKER_ENTRIES[index],
                Some(CpuId::BOOT),
                Some(CpuId::BOOT),
            );
            assert_eq!(target, CpuId::BOOT);
        }
        request_reschedule_on(CpuId::BOOT);
        drop(preempt_guard);

        wait_for_workers(2);
        verify_worker_results(2, COOPERATIVE_VERIFY_ITERATIONS);
        context_switches()
            .checked_sub(cooperative_before)
            .expect("context switch counter moved backwards during peer stress")
    };

    verify_ipi_delivery(cpu_count);
    idle_verify::verify(cpu_count);
    verify_work_stealing(cpu_count);
    synchronize_retired_tasks();
    crate::heap::shrink();

    let switches = context_switches()
        .checked_sub(switches_before)
        .expect("context switch counter moved backwards");
    let pages_after = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable after scheduler verification");
    let (idle_enters, idle_exits) = idle_counter_totals();

    assert!(
        cooperative_switches >= COOPERATIVE_MINIMUM_SWITCHES,
        "too few cooperative context switches: actual={cooperative_switches} minimum={COOPERATIVE_MINIMUM_SWITCHES}",
    );
    assert_eq!(
        pages_before,
        pages_after,
        "kernel task resources leaked: active_cpus={} retired_tasks={}",
        active_cpu_count(),
        retired_task_count(),
    );
    assert!(crate::arch::interrupt::are_enabled());

    m4c_verify::verify();
    run_verifier_thread(m4c2_verify::verify);

    crate::println!("kernel scheduler test:");
    crate::println!(" kernel threads : verified ({})", topology_worker_count,);
    crate::println!(" private stacks : verified");
    crate::println!(" context switch : verified ({} switches)", switches);
    crate::println!(
        " runnable peers : verified ({} committed switches)",
        cooperative_switches,
    );
    crate::println!(" cooperative : verified");
    crate::println!(" timer coexistence: verified");
    crate::println!(" task exit : verified");
    crate::println!(" resource reclaim: verified");
    crate::println!(
        " idle protocol : enters={} exits={}",
        idle_enters,
        idle_exits,
    );
    crate::println!("SMP scheduler test:");
    crate::println!(" participating CPUs : {}", cpu_count);
    crate::println!(" concurrent threads : verified");
    crate::println!(" per-CPU current : verified");
    crate::println!(" task affinity : verified");
    if cpu_count > 1 {
        crate::println!(" remote wakeup : verified");
        crate::println!(" IPI delivery : verified");
        crate::println!(" work stealing : verified (runnable task migration)");
    } else {
        crate::println!(" remote wakeup : single-CPU fallback");
        crate::println!(" IPI delivery : single-CPU fallback");
        crate::println!(" work stealing : single-CPU fallback");
    }
    crate::println!(" idle fallback : verified");
    crate::println!(" resource reclaim : verified");
    crate::println!("SMP_TEST: PASS");
}
