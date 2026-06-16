//! M9-A Linux-like process/thread ownership.
//!
//! M8 kept one `UserMm` inside a synchronous verifier session. M9-A moves that
//! same, already-verified address space under process ownership without changing
//! its ASID, active-CPU, page-fault, or TLB-retirement implementation.
//!
//! `Thread` owns an `Arc<Process>`. The process thread group stores only thread
//! IDs, so the ownership graph cannot form a strong-reference cycle.

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicIsize, AtomicU8, AtomicU64, AtomicUsize, Ordering};

use myos_mm::{VirtAddr, VirtRange};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::task::{Completion, TaskId};
use crate::user_mm::{UserMm, UserMmRuntimeError};

const THREAD_READY: u8 = 0;
const THREAD_RUNNABLE: u8 = 1;
const THREAD_RUNNING: u8 = 2;
const THREAD_EXITING: u8 = 3;
const THREAD_EXITED: u8 = 4;
const UNBOUND_SCHEDULER_TASK: usize = usize::MAX;

const PROCESS_THREAD_GROUP_LOCK: LockClass =
    LockClass::new("process.thread_group", LockRank::Process, 0);
const THREAD_TRAP_FRAME_LOCK: LockClass = LockClass::new("thread.trap_frame", LockRank::Process, 1);

static NEXT_PROCESS_ID: AtomicUsize = AtomicUsize::new(1);
static LIVE_PROCESSES: AtomicUsize = AtomicUsize::new(0);
static LIVE_THREADS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProcessId(usize);

impl ProcessId {
    pub const fn get(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ThreadId(usize);

impl ThreadId {
    pub const fn get(self) -> usize {
        self.0
    }
}

/// Process-wide descriptor-table anchor.
///
/// M11 will replace the allocation hint with actual file references. Keeping
/// the object process-owned now prevents syscall work from regressing to a
/// global descriptor table.
pub struct FileTable {
    next_fd_hint: AtomicUsize,
}

impl FileTable {
    const fn new() -> Self {
        Self {
            next_fd_hint: AtomicUsize::new(0),
        }
    }

    pub fn next_fd_hint(&self) -> usize {
        self.next_fd_hint.load(Ordering::Acquire)
    }
}

/// Process-wide pending-signal anchor. Per-thread masks live in `Thread`.
pub struct SignalState {
    pending: AtomicU64,
}

impl SignalState {
    const fn new() -> Self {
        Self {
            pending: AtomicU64::new(0),
        }
    }

    pub fn pending(&self) -> u64 {
        self.pending.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Credentials {
    real_uid: u32,
    effective_uid: u32,
    real_gid: u32,
    effective_gid: u32,
}

impl Credentials {
    const fn bootstrap() -> Self {
        Self {
            real_uid: 0,
            effective_uid: 0,
            real_gid: 0,
            effective_gid: 0,
        }
    }

    pub const fn real_uid(self) -> u32 {
        self.real_uid
    }

    pub const fn effective_uid(self) -> u32 {
        self.effective_uid
    }

    pub const fn real_gid(self) -> u32 {
        self.real_gid
    }

    pub const fn effective_gid(self) -> u32 {
        self.effective_gid
    }
}

/// Process-wide root and current-directory anchors.
///
/// They remain opaque until the VFS introduces a reference-counted path type.
pub struct FsContext {
    root_anchor: AtomicUsize,
    cwd_anchor: AtomicUsize,
}

impl FsContext {
    const fn bootstrap() -> Self {
        Self {
            root_anchor: AtomicUsize::new(0),
            cwd_anchor: AtomicUsize::new(0),
        }
    }

    pub fn root_anchor(&self) -> usize {
        self.root_anchor.load(Ordering::Acquire)
    }

    pub fn cwd_anchor(&self) -> usize {
        self.cwd_anchor.load(Ordering::Acquire)
    }
}

struct ThreadGroup {
    leader: Option<ThreadId>,
    members: Vec<ThreadId>,
}

impl ThreadGroup {
    const fn new() -> Self {
        Self {
            leader: None,
            members: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum ProcessError {
    AlreadyHasLeader,
    InvalidUserContext,
    MetadataOutOfMemory,
    ThreadAlreadyExited,
    ThreadNotFound,
    ThreadNotReady,
}

impl From<ProcessError> for UserMmRuntimeError {
    fn from(error: ProcessError) -> Self {
        match error {
            ProcessError::MetadataOutOfMemory => Self::MetadataOutOfMemory,
            ProcessError::AlreadyHasLeader
            | ProcessError::InvalidUserContext
            | ProcessError::ThreadAlreadyExited
            | ProcessError::ThreadNotFound
            | ProcessError::ThreadNotReady => Self::InvalidRange,
        }
    }
}

pub struct Process {
    id: ProcessId,
    mm: Arc<UserMm>,
    files: FileTable,
    signals: SignalState,
    credentials: Credentials,
    fs: FsContext,
    thread_group: IrqSpinLock<ThreadGroup>,
}

impl Process {
    pub fn create(mm: Box<UserMm>) -> Arc<Self> {
        let id = ProcessId(allocate_process_id());
        let process = Arc::new(Self {
            id,
            mm: Arc::from(mm),
            files: FileTable::new(),
            signals: SignalState::new(),
            credentials: Credentials::bootstrap(),
            fs: FsContext::bootstrap(),
            thread_group: IrqSpinLock::new_with_class(
                ThreadGroup::new(),
                PROCESS_THREAD_GROUP_LOCK,
            ),
        });
        LIVE_PROCESSES.fetch_add(1, Ordering::AcqRel);
        process
    }

    pub const fn id(&self) -> ProcessId {
        self.id
    }

    pub fn mm(&self) -> &UserMm {
        self.mm.as_ref()
    }

    pub(crate) fn mm_arc(&self) -> Arc<UserMm> {
        Arc::clone(&self.mm)
    }

    pub fn files(&self) -> &FileTable {
        &self.files
    }

    pub fn signals(&self) -> &SignalState {
        &self.signals
    }

    pub const fn credentials(&self) -> Credentials {
        self.credentials
    }

    pub fn fs(&self) -> &FsContext {
        &self.fs
    }

    pub fn thread_count(&self) -> usize {
        self.thread_group.lock().members.len()
    }

    /// Creates the thread-group leader. Linux uses the same numeric task ID for
    /// the PID and TID of the leader, so M9 follows that rule from the start.
    pub fn create_initial_thread(
        self: &Arc<Self>,
        entry: VirtAddr,
        user_stack: VirtRange,
    ) -> Result<Arc<Thread>, ProcessError> {
        let user_range = crate::arch::memory::layout::USER_RANGE;
        if !user_range.contains(entry)
            || user_stack.is_empty()
            || !user_range.contains_range(user_stack)
        {
            return Err(ProcessError::InvalidUserContext);
        }

        let mut group = self.thread_group.lock();
        if group.leader.is_some() || !group.members.is_empty() {
            return Err(ProcessError::AlreadyHasLeader);
        }
        group
            .members
            .try_reserve(1)
            .map_err(|_| ProcessError::MetadataOutOfMemory)?;

        let id = ThreadId(self.id.get());
        group.leader = Some(id);
        group.members.push(id);

        let thread = Arc::new(Thread {
            id,
            process: Arc::clone(self),
            user_pc: AtomicUsize::new(entry.get()),
            user_stack,
            trap_frame: IrqSpinLock::new_with_class(None, THREAD_TRAP_FRAME_LOCK),
            tls: AtomicUsize::new(0),
            blocked_signals: AtomicU64::new(0),
            scheduler_task: AtomicUsize::new(UNBOUND_SCHEDULER_TASK),
            visited_cpus: AtomicUsize::new(0),
            schedule_count: AtomicUsize::new(0),
            lifecycle: AtomicU8::new(THREAD_READY),
            exit_status: AtomicIsize::new(0),
            exited: Completion::new(),
        });
        LIVE_THREADS.fetch_add(1, Ordering::AcqRel);
        Ok(thread)
    }

    /// Consumes the final unique process owner and tears down its address space.
    ///
    /// The process lock is released before entering `UserMm::destroy()`, so no
    /// Process-ranked lock is held while the VM lock, page-table lock, allocator,
    /// or TLB completion paths execute.
    pub fn destroy(self) -> Result<(), UserMmRuntimeError> {
        {
            let group = self.thread_group.lock();
            assert!(
                group.members.is_empty() && group.leader.is_none(),
                "M9-B attempted to destroy a Process with live thread-group members",
            );
        }

        let Self { mm, .. } = self;
        let mut mm = Arc::try_unwrap(mm)
            .unwrap_or_else(|_| panic!("M9-B address space retained an unexpected owner"));
        mm.destroy()?;
        LIVE_PROCESSES.fetch_sub(1, Ordering::AcqRel);
        Ok(())
    }

    fn detach_thread(&self, id: ThreadId) -> Result<(), ProcessError> {
        let mut group = self.thread_group.lock();
        let index = group
            .members
            .iter()
            .position(|candidate| *candidate == id)
            .ok_or(ProcessError::ThreadNotFound)?;
        group.members.swap_remove(index);
        if group.leader == Some(id) {
            group.leader = None;
        }
        Ok(())
    }
}

pub struct Thread {
    id: ThreadId,
    process: Arc<Process>,
    user_pc: AtomicUsize,
    user_stack: VirtRange,
    trap_frame: IrqSpinLock<Option<crate::arch::trap::TrapFrame>>,
    tls: AtomicUsize,
    blocked_signals: AtomicU64,
    scheduler_task: AtomicUsize,
    visited_cpus: AtomicUsize,
    schedule_count: AtomicUsize,
    lifecycle: AtomicU8,
    exit_status: AtomicIsize,
    exited: Completion,
}

impl Thread {
    pub const fn id(&self) -> ThreadId {
        self.id
    }

    pub fn process(&self) -> &Process {
        self.process.as_ref()
    }

    pub fn entry(&self) -> VirtAddr {
        VirtAddr::new(self.user_pc.load(Ordering::Acquire))
    }

    pub const fn user_stack(&self) -> VirtRange {
        self.user_stack
    }

    pub fn prepare_entry(&self, entry: VirtAddr) -> Result<(), ProcessError> {
        if self.lifecycle.load(Ordering::Acquire) != THREAD_READY {
            return Err(ProcessError::ThreadNotReady);
        }
        if !crate::arch::memory::layout::USER_RANGE.contains(entry) {
            return Err(ProcessError::InvalidUserContext);
        }
        self.user_pc.store(entry.get(), Ordering::Release);
        Ok(())
    }

    pub(crate) fn bind_scheduler_task(&self, task_id: TaskId) -> Result<(), ProcessError> {
        self.scheduler_task
            .compare_exchange(
                UNBOUND_SCHEDULER_TASK,
                task_id.raw(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| ProcessError::ThreadNotReady)?;

        if self
            .lifecycle
            .compare_exchange(
                THREAD_READY,
                THREAD_RUNNABLE,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            self.scheduler_task
                .compare_exchange(
                    task_id.raw(),
                    UNBOUND_SCHEDULER_TASK,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .expect("M9-B Thread task binding changed during rollback");
            return Err(ProcessError::ThreadNotReady);
        }
        Ok(())
    }

    pub(crate) fn mark_running(&self) -> Result<(), ProcessError> {
        self.lifecycle
            .compare_exchange(
                THREAD_RUNNABLE,
                THREAD_RUNNING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|_| ProcessError::ThreadNotReady)
    }

    pub fn exit(&self, status: isize) -> Result<(), ProcessError> {
        self.lifecycle
            .compare_exchange(
                THREAD_RUNNING,
                THREAD_EXITING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| ProcessError::ThreadAlreadyExited)?;
        self.exit_status.store(status, Ordering::Relaxed);
        self.lifecycle.store(THREAD_EXITED, Ordering::Release);
        self.exited.complete_all();
        Ok(())
    }

    pub fn exit_status(&self) -> Option<isize> {
        if self.lifecycle.load(Ordering::Acquire) == THREAD_EXITED {
            Some(self.exit_status.load(Ordering::Relaxed))
        } else {
            None
        }
    }

    pub fn tls(&self) -> usize {
        self.tls.load(Ordering::Acquire)
    }

    pub fn blocked_signals(&self) -> u64 {
        self.blocked_signals.load(Ordering::Acquire)
    }

    #[allow(dead_code)]
    pub fn set_tls(&self, value: usize) {
        self.tls.store(value, Ordering::Release);
    }

    #[allow(dead_code)]
    pub fn set_blocked_signals(&self, mask: u64) {
        self.blocked_signals.store(mask, Ordering::Release);
    }

    pub fn scheduler_task(&self) -> Option<TaskId> {
        let task = self.scheduler_task.load(Ordering::Acquire);
        if task == UNBOUND_SCHEDULER_TASK {
            None
        } else {
            Some(TaskId::from_raw(task))
        }
    }

    pub(crate) fn record_cpu(&self, cpu: crate::smp::CpuId) {
        let bit = 1_usize
            .checked_shl(u32::try_from(cpu.get()).expect("CPU index does not fit u32"))
            .expect("CPU index exceeds thread CPU mask width");
        self.visited_cpus.fetch_or(bit, Ordering::AcqRel);
        self.schedule_count.fetch_add(1, Ordering::AcqRel);
    }

    pub fn visited_cpu_mask(&self) -> usize {
        self.visited_cpus.load(Ordering::Acquire)
    }

    pub fn schedule_count(&self) -> usize {
        self.schedule_count.load(Ordering::Acquire)
    }

    pub fn wait_for_exit(&self) -> isize {
        self.exited.wait();
        self.exit_status()
            .expect("M9-B exit completion published without an exit status")
    }

    #[allow(dead_code)]
    pub fn save_trap_frame(&self, frame: crate::arch::trap::TrapFrame) {
        let mut slot = self.trap_frame.lock();
        assert!(
            slot.is_none(),
            "M9-B attempted to overwrite an unconsumed user trap frame",
        );
        *slot = Some(frame);
    }

    #[allow(dead_code)]
    pub fn take_trap_frame(&self) -> Option<crate::arch::trap::TrapFrame> {
        self.trap_frame.lock().take()
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        assert_eq!(
            self.lifecycle.load(Ordering::Acquire),
            THREAD_EXITED,
            "M9-B Thread dropped before completing exit",
        );
        assert!(
            self.trap_frame.lock().is_none(),
            "M9-B Thread dropped with an owned trap frame",
        );
        self.process
            .detach_thread(self.id)
            .expect("M9-B Thread disappeared from its process thread group");
        LIVE_THREADS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn assert_initial_pair(process: &Arc<Process>, thread: &Arc<Thread>) {
    assert_eq!(thread.process().id(), process.id());
    assert_eq!(thread.id().get(), process.id().get());
    assert_eq!(process.thread_count(), 1);
    assert_eq!(Arc::strong_count(process), 2);
    assert_eq!(Arc::strong_count(&process.mm), 1);
    assert_eq!(process.files().next_fd_hint(), 0);
    assert_eq!(process.signals().pending(), 0);
    assert_eq!(process.credentials().real_uid(), 0);
    assert_eq!(process.credentials().effective_uid(), 0);
    assert_eq!(process.credentials().real_gid(), 0);
    assert_eq!(process.credentials().effective_gid(), 0);
    assert_eq!(process.fs().root_anchor(), 0);
    assert_eq!(process.fs().cwd_anchor(), 0);
    assert_eq!(thread.tls(), 0);
    assert_eq!(thread.blocked_signals(), 0);
    assert!(thread.exit_status().is_none());
    assert_eq!(thread.visited_cpu_mask(), 0);
    assert_eq!(thread.schedule_count(), 0);
}

pub fn assert_no_leaks() {
    assert_eq!(
        LIVE_THREADS.load(Ordering::Acquire),
        0,
        "M9-A leaked a Thread object",
    );
    assert_eq!(
        LIVE_PROCESSES.load(Ordering::Acquire),
        0,
        "M9-A leaked a Process object",
    );
}

fn allocate_process_id() -> usize {
    NEXT_PROCESS_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("M9-A exhausted the process-ID namespace")
}
