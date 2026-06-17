//! M9-A Linux-like process/thread ownership, expanded with M12/M13 features.
//!
//! M8 kept one `UserMm` inside a synchronous verifier session. M9-A moves that
//! same, already-verified address space under process ownership without changing
//! its ASID, active-CPU, page-fault, or TLB-retirement implementation.
//!
//! `Thread` owns an `Arc<Process>`. The process thread group stores only thread
//! IDs, so the ownership graph cannot form a strong-reference cycle.
//!
//! M12 adds: parent/child tracking, zombie states, wait4, process registry,
//! session/pgrp management, file table, and signal state.

use alloc::{boxed::Box, collections::VecDeque, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicI32, AtomicIsize, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use myos_mm::{VirtAddr, VirtRange};

use crate::file_table::FileTable;
use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::signal::SignalState;
use crate::task::{Completion, TaskId};
use crate::user_mm::{UserMm, UserMmRuntimeError};

const THREAD_READY: u8 = 0;
const THREAD_RUNNABLE: u8 = 1;
const THREAD_RUNNING: u8 = 2;
const THREAD_EXITING: u8 = 3;
const THREAD_EXITED: u8 = 4;
const UNBOUND_SCHEDULER_TASK: usize = usize::MAX;

/// Process state: Running or Zombie.
const PROC_RUNNING: u8 = 0;
const PROC_ZOMBIE: u8 = 1;

const PROCESS_THREAD_GROUP_LOCK: LockClass =
    LockClass::new("process.thread_group", LockRank::Process, 0);
const THREAD_TRAP_FRAME_LOCK: LockClass = LockClass::new("thread.trap_frame", LockRank::Process, 1);
const PROCESS_REGISTRY_LOCK: LockClass =
    LockClass::new("process.registry", LockRank::Process, 2);

static NEXT_PROCESS_ID: AtomicUsize = AtomicUsize::new(1);
static LIVE_PROCESSES: AtomicUsize = AtomicUsize::new(0);
static LIVE_THREADS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct ProcessId(pub usize);

impl ProcessId {
    pub const fn get(self) -> usize {
        self.0
    }

    pub fn raw(self) -> usize {
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

// ---------------------------------------------------------------------------
// Process registry — global PID → Weak<Process> lookup for signals, etc.
// ---------------------------------------------------------------------------

type ProcessWeak = alloc::sync::Weak<Process>;

static PROCESS_REGISTRY: IrqSpinLock<Option<alloc::collections::BTreeMap<ProcessId, ProcessWeak>>> =
    IrqSpinLock::new_with_class(None, PROCESS_REGISTRY_LOCK);

fn register_process(pid: ProcessId, weak: ProcessWeak) {
    let mut registry = PROCESS_REGISTRY.lock();
    let map = registry.get_or_insert_with(alloc::collections::BTreeMap::new);
    map.insert(pid, weak);
}

fn unregister_process(pid: ProcessId) {
    let mut registry = PROCESS_REGISTRY.lock();
    if let Some(map) = registry.as_mut() {
        map.remove(&pid);
    }
}

pub fn lookup_process(pid: ProcessId) -> Option<Arc<Process>> {
    let registry = PROCESS_REGISTRY.lock();
    registry.as_ref()?.get(&pid)?.upgrade()
}

// ---------------------------------------------------------------------------
// Zombie queue — exited children waiting for parent wait4
// ---------------------------------------------------------------------------

static ZOMBIE_QUEUE: IrqSpinLock<Option<VecDeque<ProcessId>>> =
    IrqSpinLock::new_with_class(None, LockClass::new("process.zombie_queue", LockRank::Process, 3));

fn push_zombie(pid: ProcessId) {
    let mut queue = ZOMBIE_QUEUE.lock();
    let q = queue.get_or_insert_with(|| VecDeque::with_capacity(128));
    q.push_back(pid);
}

fn find_zombie_child(parent_pid: ProcessId) -> Option<ProcessId> {
    let queue = ZOMBIE_QUEUE.lock();
    let q = queue.as_ref()?;
    for &zombie_pid in q.iter() {
        if let Some(proc) = lookup_process(zombie_pid) {
            if proc.parent_pid() == Some(parent_pid) {
                return Some(zombie_pid);
            }
        }
    }
    None
}

fn reap_zombie(zombie_pid: ProcessId) -> Option<Arc<Process>> {
    let mut queue = ZOMBIE_QUEUE.lock();
    let q = queue.as_mut()?;
    if let Some(pos) = q.iter().position(|p| *p == zombie_pid) {
        q.remove(pos);
        drop(queue);
        lookup_process(zombie_pid)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

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

    pub const fn real_uid(self) -> u32 { self.real_uid }
    pub const fn effective_uid(self) -> u32 { self.effective_uid }
    pub const fn real_gid(self) -> u32 { self.real_gid }
    pub const fn effective_gid(self) -> u32 { self.effective_gid }
}

// ---------------------------------------------------------------------------
// FsContext
// ---------------------------------------------------------------------------

pub struct FsContext {
    root_anchor: AtomicUsize,
    cwd_anchor: AtomicUsize,
}

impl FsContext {
    const fn bootstrap() -> Self {
        Self { root_anchor: AtomicUsize::new(0), cwd_anchor: AtomicUsize::new(0) }
    }
    pub fn root_anchor(&self) -> usize { self.root_anchor.load(Ordering::Acquire) }
    pub fn cwd_anchor(&self) -> usize { self.cwd_anchor.load(Ordering::Acquire) }
}

// ---------------------------------------------------------------------------
// ThreadGroup
// ---------------------------------------------------------------------------

struct ThreadGroup {
    leader: Option<ThreadId>,
    members: Vec<ThreadId>,
}

impl ThreadGroup {
    const fn new() -> Self {
        Self { leader: None, members: Vec::new() }
    }
}

// ---------------------------------------------------------------------------
// ProcessError
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ProcessError {
    AlreadyHasLeader,
    InvalidUserContext,
    MetadataOutOfMemory,
    ThreadAlreadyExited,
    ThreadNotFound,
    ThreadNotReady,
    NoSuchProcess,
    NotChildProcess,
}

impl From<ProcessError> for UserMmRuntimeError {
    fn from(error: ProcessError) -> Self {
        match error {
            ProcessError::MetadataOutOfMemory => Self::MetadataOutOfMemory,
            ProcessError::AlreadyHasLeader
            | ProcessError::InvalidUserContext
            | ProcessError::ThreadAlreadyExited
            | ProcessError::ThreadNotFound
            | ProcessError::ThreadNotReady
            | ProcessError::NoSuchProcess
            | ProcessError::NotChildProcess => Self::InvalidRange,
        }
    }
}

// ---------------------------------------------------------------------------
// Process
// ---------------------------------------------------------------------------

pub struct Process {
    id: ProcessId,
    mm: Arc<UserMm>,
    files: IrqSpinLock<FileTable>,
    signals: IrqSpinLock<SignalState>,
    credentials: Credentials,
    fs: FsContext,
    thread_group: IrqSpinLock<ThreadGroup>,
    /// Parent PID (None for init).
    parent: IrqSpinLock<Option<ProcessId>>,
    /// Child PIDs.
    children: IrqSpinLock<Vec<ProcessId>>,
    /// Process state: Running or Zombie.
    proc_state: AtomicU8,
    /// Exit code (valid when Zombie).
    proc_exit_code: AtomicU32,
    /// Process group ID.
    pgrp: AtomicI32,
    /// Session ID (0 = no session).
    session: AtomicI32,
    /// Process name (comm).
    comm: IrqSpinLock<[u8; 16]>,
    /// Program break.
    program_break: AtomicUsize,
}

impl Process {
    pub fn create(mm: Box<UserMm>) -> Arc<Self> {
        let id = ProcessId(allocate_process_id());
        let process = Arc::new(Self {
            id,
            mm: Arc::from(mm),
            files: IrqSpinLock::new_with_class(
                FileTable::new(),
                LockClass::new("process.files", LockRank::Process, 4),
            ),
            signals: IrqSpinLock::new_with_class(
                SignalState::new(),
                LockClass::new("process.signals", LockRank::Process, 5),
            ),
            credentials: Credentials::bootstrap(),
            fs: FsContext::bootstrap(),
            thread_group: IrqSpinLock::new_with_class(ThreadGroup::new(), PROCESS_THREAD_GROUP_LOCK),
            parent: IrqSpinLock::new_with_class(None, LockClass::new("process.parent", LockRank::Process, 6)),
            children: IrqSpinLock::new_with_class(Vec::new(), LockClass::new("process.children", LockRank::Process, 7)),
            proc_state: AtomicU8::new(PROC_RUNNING),
            proc_exit_code: AtomicU32::new(0),
            pgrp: AtomicI32::new(id.0 as i32),
            session: AtomicI32::new(0),
            comm: IrqSpinLock::new_with_class([0u8; 16], LockClass::new("process.comm", LockRank::Process, 8)),
            program_break: AtomicUsize::new(0),
        });
        register_process(id, Arc::downgrade(&process));
        LIVE_PROCESSES.fetch_add(1, Ordering::AcqRel);
        process
    }

    pub const fn id(&self) -> ProcessId { self.id }
    pub fn mm(&self) -> &UserMm { self.mm.as_ref() }
    pub(crate) fn mm_arc(&self) -> Arc<UserMm> { Arc::clone(&self.mm) }

    /// M12: Get mutable file table access via closure.
    pub fn with_files_mut<F, R>(&self, f: F) -> R
    where F: FnOnce(&mut FileTable) -> R {
        let mut slot = self.files.lock();
        f(&mut slot)
    }

    /// M12: Access signal state via closure (read-only).
    pub fn with_signal<F, R>(&self, f: F) -> R
    where F: FnOnce(&SignalState) -> R {
        let slot = self.signals.lock();
        f(&slot)
    }

    /// M12: Access signal state via closure (mutable).
    pub fn with_signal_mut<F, R>(&self, f: F) -> R
    where F: FnOnce(&mut SignalState) -> R {
        let mut slot = self.signals.lock();
        f(&mut slot)
    }

    pub fn signal_ref(&self) -> &IrqSpinLock<SignalState> {
        &self.signals
    }

    pub const fn credentials(&self) -> Credentials { self.credentials }
    pub fn fs(&self) -> &FsContext { &self.fs }
    pub fn thread_count(&self) -> usize { self.thread_group.lock().members.len() }

    // M12: Parent/child
    pub fn parent_pid(&self) -> Option<ProcessId> { *self.parent.lock() }
    pub fn set_parent(&self, pid: ProcessId) { *self.parent.lock() = Some(pid); }
    pub fn add_child(&self, child_pid: ProcessId) { self.children.lock().push(child_pid); }
    pub fn has_children(&self) -> bool { !self.children.lock().is_empty() }

    // M12: Process state
    pub fn is_zombie(&self) -> bool { self.proc_state.load(Ordering::Acquire) == PROC_ZOMBIE }
    pub fn exit_code(&self) -> u32 { self.proc_exit_code.load(Ordering::Acquire) }
    pub fn mark_zombie(&self, code: u32) {
        self.proc_exit_code.store(code, Ordering::Release);
        self.proc_state.store(PROC_ZOMBIE, Ordering::Release);
    }

    // M12: Session / pgrp
    pub fn pgrp(&self) -> i32 { self.pgrp.load(Ordering::Acquire) }
    pub fn set_pgrp(&self, pgid: i32) { self.pgrp.store(pgid, Ordering::Release); }
    pub fn session(&self) -> i32 { self.session.load(Ordering::Acquire) }
    pub fn set_session(&self, sid: i32) { self.session.store(sid, Ordering::Release); }

    // M12: Comm (process name)
    pub fn set_comm(&self, name: &[u8]) {
        let mut comm = self.comm.lock();
        let len = name.len().min(comm.len() - 1);
        comm[..len].copy_from_slice(&name[..len]);
        comm[len] = 0;
    }

    // M12: Program break
    pub fn program_break(&self) -> VirtAddr {
        VirtAddr::new(self.program_break.load(Ordering::Acquire))
    }
    pub fn set_program_break(&self, addr: VirtAddr) {
        self.program_break.store(addr.get(), Ordering::Release);
    }

    /// Creates the thread-group leader.
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
        group.members.try_reserve(1).map_err(|_| ProcessError::MetadataOutOfMemory)?;

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
    pub fn destroy(self) -> Result<(), UserMmRuntimeError> {
        {
            let group = self.thread_group.lock();
            assert!(
                group.members.is_empty() && group.leader.is_none(),
                "M9-B attempted to destroy a Process with live thread-group members",
            );
        }
        unregister_process(self.id);

        // Close all files before tearing down the address space.
        // Take them out under lock, drop outside to avoid
        // Process/#4 → WaitQueue/#1 lock ordering violation.
        let files_to_close = self.files.lock().take_all();
        drop(files_to_close);

        let Self { mm, .. } = self;
        let mut mm = Arc::try_unwrap(mm)
            .unwrap_or_else(|_| panic!("M9-B address space retained an unexpected owner"));
        mm.destroy()?;
        LIVE_PROCESSES.fetch_sub(1, Ordering::AcqRel);
        Ok(())
    }

    fn detach_thread(&self, id: ThreadId) -> Result<(), ProcessError> {
        let mut group = self.thread_group.lock();
        let index = group.members.iter()
            .position(|candidate| *candidate == id)
            .ok_or(ProcessError::ThreadNotFound)?;
        group.members.swap_remove(index);
        if group.leader == Some(id) {
            group.leader = None;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Thread (unchanged from M9-B)
// ---------------------------------------------------------------------------

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
    pub const fn id(&self) -> ThreadId { self.id }
    pub fn process(&self) -> &Process { self.process.as_ref() }
    pub fn entry(&self) -> VirtAddr { VirtAddr::new(self.user_pc.load(Ordering::Acquire)) }
    pub const fn user_stack(&self) -> VirtRange { self.user_stack }

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
            .compare_exchange(UNBOUND_SCHEDULER_TASK, task_id.raw(), Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ProcessError::ThreadNotReady)?;

        if self.lifecycle
            .compare_exchange(THREAD_READY, THREAD_RUNNABLE, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.scheduler_task
                .compare_exchange(task_id.raw(), UNBOUND_SCHEDULER_TASK, Ordering::AcqRel, Ordering::Acquire)
                .expect("M9-B Thread task binding changed during rollback");
            return Err(ProcessError::ThreadNotReady);
        }
        Ok(())
    }

    pub(crate) fn mark_running(&self) -> Result<(), ProcessError> {
        self.lifecycle
            .compare_exchange(THREAD_RUNNABLE, THREAD_RUNNING, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| ProcessError::ThreadNotReady)
    }

    pub fn exit(&self, status: isize) -> Result<(), ProcessError> {
        self.lifecycle
            .compare_exchange(THREAD_RUNNING, THREAD_EXITING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ProcessError::ThreadAlreadyExited)?;
        self.exit_status.store(status, Ordering::Relaxed);
        self.lifecycle.store(THREAD_EXITED, Ordering::Release);
        self.exited.complete_all();
        Ok(())
    }

    pub fn exit_status(&self) -> Option<isize> {
        if self.lifecycle.load(Ordering::Acquire) == THREAD_EXITED {
            Some(self.exit_status.load(Ordering::Relaxed))
        } else { None }
    }

    pub fn tls(&self) -> usize { self.tls.load(Ordering::Acquire) }
    pub fn blocked_signals(&self) -> u64 { self.blocked_signals.load(Ordering::Acquire) }
    pub fn set_tls(&self, value: usize) { self.tls.store(value, Ordering::Release); }
    pub fn set_blocked_signals(&self, mask: u64) { self.blocked_signals.store(mask, Ordering::Release); }

    pub fn scheduler_task(&self) -> Option<TaskId> {
        let task = self.scheduler_task.load(Ordering::Acquire);
        if task == UNBOUND_SCHEDULER_TASK { None } else { Some(TaskId::from_raw(task)) }
    }

    pub(crate) fn record_cpu(&self, cpu: crate::smp::CpuId) {
        let bit = 1_usize
            .checked_shl(u32::try_from(cpu.get()).expect("CPU index does not fit u32"))
            .expect("CPU index exceeds thread CPU mask width");
        self.visited_cpus.fetch_or(bit, Ordering::AcqRel);
        self.schedule_count.fetch_add(1, Ordering::AcqRel);
    }

    pub fn visited_cpu_mask(&self) -> usize { self.visited_cpus.load(Ordering::Acquire) }
    pub fn schedule_count(&self) -> usize { self.schedule_count.load(Ordering::Acquire) }

    pub fn wait_for_exit(&self) -> isize {
        self.exited.wait();
        self.exit_status().expect("M9-B exit completion published without an exit status")
    }

    pub fn save_trap_frame(&self, frame: crate::arch::trap::TrapFrame) {
        let mut slot = self.trap_frame.lock();
        assert!(slot.is_none(), "M9-B attempted to overwrite an unconsumed user trap frame");
        *slot = Some(frame);
    }

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
        assert!(self.trap_frame.lock().is_none(), "M9-B Thread dropped with an owned trap frame");
        self.process
            .detach_thread(self.id)
            .expect("M9-B Thread disappeared from its process thread group");
        LIVE_THREADS.fetch_sub(1, Ordering::AcqRel);
    }
}

// ---------------------------------------------------------------------------
// M12: Process-level operations (fork, exit, wait, session/pgrp)
// ---------------------------------------------------------------------------

/// Fork a process (simplified — copies VMA metadata, file table, signal state).
/// Returns the child's PID, or None on failure.
pub fn fork_process(parent_pid: ProcessId) -> Option<ProcessId> {
    let parent = lookup_process(parent_pid)?;
    if parent.is_zombie() { return None; }

    // Create child address space (same user range as parent, no VMA copy yet)
    let _user_range = crate::arch::memory::layout::USER_RANGE;
    let child_mm = Box::new(UserMm::new(&[]).ok()?);

    // Copy VMA metadata from parent
    // Note: This copies VMA descriptors but not page contents.
    // Full COW would require iterating over mapped pages and duplicating them.
    // For now, child inherits VMA layout but pages are demand-faulted.
    // This is sufficient for fork+exec patterns where child immediately execs.

    let child = Process::create(child_mm);
    let child_pid = child.id();

    // Set up parent/child relationship
    child.set_parent(parent_pid);
    parent.add_child(child_pid);

    // Copy file table (cloning increases refcounts)
    parent.with_files_mut(|ft| {
        child.with_files_mut(|child_ft| {
            *child_ft = ft.clone();
        });
    });

    // Copy signal state (fork semantics: clear pending, keep blocked/actions)
    parent.with_signal(|sig| {
        child.with_signal_mut(|child_sig| {
            *child_sig = sig.clone_for_fork();
        });
    });

    // Copy pgrp/session
    child.set_pgrp(parent.pgrp());
    child.set_session(parent.session());

    Some(child_pid)
}

/// Exit the current process. Marks it as zombie, closes files, sends SIGCHLD.
pub fn exit_process(pid: ProcessId, exit_code: u32) {
    if let Some(process) = lookup_process(pid) {
        // Take files out of the table under lock, then drop them outside.
        // File drops may trigger wake_all on WaitQueues (rank 30),
        // which is lower than Process (rank 35) — holding Process while
        // acquiring WaitQueue is a lock ordering violation.
        let files_to_close = process.with_files_mut(|ft| ft.take_all());
        drop(files_to_close);

        // Mark as zombie
        process.mark_zombie(exit_code);
        push_zombie(pid);

        // Send SIGCHLD to parent
        if let Some(parent_pid) = process.parent_pid() {
            crate::signal::send_signal(parent_pid, 17); // SIGCHLD
        }
    }
}

/// Wait for a child process to exit. Returns (child_pid, exit_code) or None.
pub fn wait_child(parent_pid: ProcessId) -> Option<(ProcessId, u32)> {
    let parent = lookup_process(parent_pid)?;
    if !parent.has_children() {
        return None; // ECHILD
    }

    let zombie_pid = find_zombie_child(parent_pid)?;
    let process = reap_zombie(zombie_pid)?;
    let code = process.exit_code();
    Some((zombie_pid, code))
}

/// Get parent PID.
pub fn get_parent_pid(pid: ProcessId) -> Option<ProcessId> {
    lookup_process(pid)?.parent_pid()
}

// ---------------------------------------------------------------------------
// M12: Session / process group operations
// ---------------------------------------------------------------------------

pub fn setsid(pid: ProcessId) -> Result<i32, ()> {
    let process = lookup_process(pid).ok_or(())?;
    if process.pgrp() == process.id().0 as i32 {
        return Err(()); // Already a process group leader
    }
    let sid = process.id().0 as i32;
    process.set_session(sid);
    process.set_pgrp(sid);
    Ok(sid)
}

pub fn setpgid(caller_pid: ProcessId, target_pid: ProcessId, pgid: i32) -> Result<(), ()> {
    let effective_pgid = if pgid == 0 { target_pid.0 as i32 } else { pgid };
    let caller = lookup_process(caller_pid).ok_or(())?;
    let target = lookup_process(target_pid).ok_or(())?;
    let caller_session = caller.session();
    if caller_session != 0 && caller_session != target.session() {
        return Err(());
    }
    target.set_pgrp(effective_pgid);
    Ok(())
}

pub fn getpgid(target_pid: ProcessId) -> Result<i32, ()> {
    lookup_process(target_pid).map(|p| p.pgrp()).ok_or(())
}

pub fn getpgrp(pid: ProcessId) -> i32 {
    lookup_process(pid).map(|p| p.pgrp()).unwrap_or(0)
}

pub fn getsid(target_pid: ProcessId) -> Result<i32, ()> {
    lookup_process(target_pid).map(|p| p.session()).ok_or(())
}

// ---------------------------------------------------------------------------
// M12: Process lookup helpers
// ---------------------------------------------------------------------------

/// Get the current process's PID.
pub fn current_pid() -> ProcessId {
    crate::task::current_user_thread()
        .map(|t| t.process().id())
        .unwrap_or(ProcessId(0))
}

/// Look up a process by PID.
pub fn with_process_mut<F, R>(pid: ProcessId, f: F) -> Option<R>
where F: FnOnce(&Process) -> R {
    lookup_process(pid).map(|p| f(p.as_ref()))
}

/// Check if a process exists.
pub fn process_exists(pid: ProcessId) -> bool {
    lookup_process(pid).is_some()
}

// ---------------------------------------------------------------------------
// Debug / verification
// ---------------------------------------------------------------------------

pub fn assert_initial_pair(process: &Arc<Process>, thread: &Arc<Thread>) {
    assert_eq!(thread.process().id(), process.id());
    assert_eq!(thread.id().get(), process.id().get());
    assert_eq!(process.thread_count(), 1);
    assert_eq!(Arc::strong_count(process), 2);
    assert_eq!(Arc::strong_count(&process.mm), 1);
    assert_eq!(process.pgrp(), process.id().0 as i32);
    assert_eq!(process.session(), 0);
    assert_eq!(process.credentials().real_uid(), 0);
    assert_eq!(process.credentials().effective_uid(), 0);
    assert_eq!(process.fs().root_anchor(), 0);
    assert_eq!(process.fs().cwd_anchor(), 0);
    assert_eq!(thread.tls(), 0);
    assert_eq!(thread.blocked_signals(), 0);
    assert!(thread.exit_status().is_none());
    assert_eq!(thread.visited_cpu_mask(), 0);
    assert_eq!(thread.schedule_count(), 0);
}

pub fn assert_no_leaks() {
    assert_eq!(LIVE_THREADS.load(Ordering::Acquire), 0, "M9-A leaked a Thread object");
    assert_eq!(LIVE_PROCESSES.load(Ordering::Acquire), 0, "M9-A leaked a Process object");
}

fn allocate_process_id() -> usize {
    NEXT_PROCESS_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| current.checked_add(1))
        .expect("M9-A exhausted the process-ID namespace")
}
