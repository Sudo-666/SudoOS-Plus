//! M9-A Linux-like process/thread ownership.
//!
//! M8 kept one `UserMm` inside a synchronous verifier session. M9-A moves that
//! same, already-verified address space under process ownership without changing
//! its ASID, active-CPU, page-fault, or TLB-retirement implementation.
//!
//! `Thread` owns an `Arc<Process>`. The process thread group stores only thread
//! IDs, so the ownership graph cannot form a strong-reference cycle.

use alloc::{boxed::Box, collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{
    AtomicBool, AtomicIsize, AtomicU8, AtomicU64, AtomicUsize, Ordering,
};

use myos_mm::{VirtAddr, VirtRange};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::task::{Completion, TaskId, WaitQueue};
use crate::user_mm::{UserMm, UserMmRuntimeError};

pub const PROCESS_MAX_FDS: usize = 128;

const THREAD_READY: u8 = 0;
const THREAD_RUNNABLE: u8 = 1;
const THREAD_RUNNING: u8 = 2;
const THREAD_EXITING: u8 = 3;
const THREAD_EXITED: u8 = 4;
const NO_FORCED_EXIT: isize = isize::MIN;
// SUDOOS_SIGNAL_PENDING_SIGSUSPEND_FIX_V1
// unblockable signal bits are always removed from a legal mask, therefore
// u64::MAX is a safe out-of-band value for "no saved sigsuspend mask".
const NO_SIGSUSPEND_SAVED_MASK: u64 = u64::MAX;
const UNBOUND_SCHEDULER_TASK: usize = usize::MAX;
const PROCESS_MM_LOCK: LockClass = LockClass::new("process.mm", LockRank::Process, 0);
const PROCESS_THREAD_GROUP_LOCK: LockClass =
    LockClass::new("process.thread_group", LockRank::Process, 1);
const THREAD_TRAP_FRAME_LOCK: LockClass = LockClass::new("thread.trap_frame", LockRank::Process, 1);
const PROCESS_SIGNAL_ACTION_LOCK: LockClass =
    LockClass::new("process.signal_action", LockRank::Process, 1);
const PROCESS_FILE_TABLE_LOCK: LockClass =
    LockClass::new("process.file_table", LockRank::Process, 2);
const PROCESS_FS_LOCK: LockClass = LockClass::new("process.fs", LockRank::Process, 3);
const PROCESS_RELATION_LOCK: LockClass = LockClass::new("process.relation", LockRank::Process, 4);
const PROCESS_REGISTRY_LOCK: LockClass = LockClass::new("process.registry", LockRank::Process, 5);

static NEXT_PROCESS_ID: AtomicUsize = AtomicUsize::new(1);
static LIVE_PROCESSES: AtomicUsize = AtomicUsize::new(0);
static LIVE_THREADS: AtomicUsize = AtomicUsize::new(0);
static PROCESS_REGISTRY: IrqSpinLock<Option<BTreeMap<ProcessId, alloc::sync::Weak<Process>>>> =
    IrqSpinLock::new_with_class(None, PROCESS_REGISTRY_LOCK);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProcessId(usize);

impl ProcessId {
    pub const fn get(self) -> usize {
        self.0
    }

    pub const fn from_raw_for_kernel(raw: usize) -> Self {
        Self(raw)
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
pub struct FileTable {
    table: IrqSpinLock<myos_vfs::FileTable<PROCESS_MAX_FDS>>,
}

impl FileTable {
    fn new() -> Self {
        Self {
            table: IrqSpinLock::new_with_class(myos_vfs::FileTable::new(), PROCESS_FILE_TABLE_LOCK),
        }
    }

    pub fn open_count(&self) -> usize {
        self.table.lock().open_count()
    }

    pub fn install_at(
        &self,
        fd: usize,
        file: myos_vfs::ArcFile,
        close_on_exec: bool,
    ) -> Result<(), myos_vfs::Errno> {
        let old = self.table.lock().replace_fd_take(fd, file, close_on_exec)?;
        drop(old);
        Ok(())
    }

    pub fn allocate(
        &self,
        file: myos_vfs::ArcFile,
        close_on_exec: bool,
    ) -> Result<usize, myos_vfs::Errno> {
        self.table.lock().allocate_fd(file, close_on_exec)
    }

    pub fn get(&self, fd: usize) -> Result<myos_vfs::ArcFile, myos_vfs::Errno> {
        self.table.lock().get_file(fd)
    }

    pub fn close(&self, fd: usize) -> Result<(), myos_vfs::Errno> {
        let file = self.table.lock().take_fd(fd)?;
        drop(file);
        Ok(())
    }

    pub fn dup_from(
        &self,
        old_fd: usize,
        min_fd: usize,
        close_on_exec: bool,
    ) -> Result<usize, myos_vfs::Errno> {
        self.table.lock().dup_from(old_fd, min_fd, close_on_exec)
    }

    pub fn dup_to(
        &self,
        old_fd: usize,
        new_fd: usize,
        close_on_exec: bool,
    ) -> Result<usize, myos_vfs::Errno> {
        let (fd, old) = self
            .table
            .lock()
            .dup_to_take(old_fd, new_fd, close_on_exec)?;
        drop(old);
        Ok(fd)
    }

    pub fn fd_flags(&self, fd: usize) -> Result<u32, myos_vfs::Errno> {
        self.table.lock().fd_flags(fd)
    }

    pub fn set_close_on_exec(&self, fd: usize, close_on_exec: bool) -> Result<(), myos_vfs::Errno> {
        self.table.lock().set_close_on_exec(fd, close_on_exec)
    }

    pub fn file_flags(&self, fd: usize) -> Result<myos_vfs::OpenFlags, myos_vfs::Errno> {
        self.table.lock().file_flags(fd)
    }

    pub fn close_on_exec(&self) -> Result<(), myos_vfs::Errno> {
        let mut files = Vec::new();
        files
            .try_reserve(PROCESS_MAX_FDS)
            .map_err(|_| myos_vfs::Errno::Enomem)?;
        self.table.lock().take_close_on_exec(&mut files);
        drop(files);
        Ok(())
    }

    pub fn close_all(&self) -> Result<(), myos_vfs::Errno> {
        let mut files = Vec::new();
        files
            .try_reserve(PROCESS_MAX_FDS)
            .map_err(|_| myos_vfs::Errno::Enomem)?;
        self.table.lock().take_all(&mut files);
        for file in &files {
            file.process_exit();
        }
        drop(files);
        Ok(())
    }
}

/// Process-wide pending-signal anchor. Per-thread masks live in `Thread`.
pub struct SignalState {
    pending: AtomicU64,
    blocked: AtomicU64,
    actions: IrqSpinLock<[crate::signal::KernelSigAction; 64]>,
    waiters: WaitQueue,
}

impl SignalState {
    fn new() -> Self {
        Self {
            pending: AtomicU64::new(0),
            blocked: AtomicU64::new(0),
            actions: IrqSpinLock::new_with_class(
                [crate::signal::KernelSigAction::default(); 64],
                PROCESS_SIGNAL_ACTION_LOCK,
            ),
            waiters: WaitQueue::new(),
        }
    }

    pub fn pending(&self) -> u64 {
        self.pending.load(Ordering::Acquire)
    }

    pub fn blocked(&self) -> u64 {
        self.blocked.load(Ordering::Acquire)
    }

    pub fn set_blocked(&self, mask: u64) {
        self.blocked.store(mask, Ordering::Release);
    }

    pub fn add_pending(&self, signal: u32) -> Result<(), myos_vfs::Errno> {
        if signal == 0 || signal >= 64 {
            return Err(myos_vfs::Errno::Einval);
        }
        let bit = 1_u64 << (signal - 1);
        self.pending.fetch_or(bit, Ordering::AcqRel);
        self.waiters.wake_all();
        Ok(())
    }

    pub const fn wait_queue(&self) -> &WaitQueue {
        &self.waiters
    }

    pub fn action(&self, signal: u32) -> Option<crate::signal::KernelSigAction> {
        signal
            .checked_sub(1)
            .and_then(|index| self.actions.lock().get(index as usize).copied())
    }

    pub fn set_action(
        &self,
        signal: u32,
        action: crate::signal::KernelSigAction,
    ) -> Result<(), myos_vfs::Errno> {
        let index = signal.checked_sub(1).ok_or(myos_vfs::Errno::Einval)? as usize;
        let mut actions = self.actions.lock();
        let slot = actions.get_mut(index).ok_or(myos_vfs::Errno::Einval)?;
        *slot = action;
        Ok(())
    }

    pub fn copy_actions_from(&self, parent: &Self) {
        for signal in 1..64 {
            let action = parent
                .action(signal)
                .expect("parent signal action table lost a valid signal slot");
            self.set_action(signal, action)
                .expect("child signal action table lost a valid signal slot");
        }
    }

    pub fn reset_actions_for_exec(&self) {
        const SIG_IGN: usize = 1;
        let mut actions = self.actions.lock();
        for action in actions.iter_mut() {
            if action.handler != SIG_IGN {
                *action = crate::signal::KernelSigAction::default();
            }
        }
    }

    pub fn take_matching_unblocked(&self, wanted: u64, blocked: u64) -> Option<u32> {
        loop {
            let pending = self.pending.load(Ordering::Acquire);
            let available = pending & !blocked & wanted;
            if available == 0 {
                return None;
            }
            let signal = available.trailing_zeros() + 1;
            let bit = 1_u64 << (signal - 1);
            if self
                .pending
                .compare_exchange(pending, pending & !bit, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(signal);
            }
        }
    }

    pub fn take_unblocked(&self, blocked: u64) -> Option<u32> {
        loop {
            let pending = self.pending.load(Ordering::Acquire);
            let available = pending & !blocked;
            if available == 0 {
                return None;
            }
            let signal = available.trailing_zeros() + 1;
            let bit = 1_u64 << (signal - 1);
            if self
                .pending
                .compare_exchange(pending, pending & !bit, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(signal);
            }
        }
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

/// Process-wide root and current-directory paths.
pub struct FsContext {
    root: IrqSpinLock<String>,
    cwd: IrqSpinLock<String>,
}

impl FsContext {
    fn bootstrap() -> Self {
        Self {
            root: IrqSpinLock::new_with_class(String::from("/"), PROCESS_FS_LOCK),
            cwd: IrqSpinLock::new_with_class(String::from("/"), PROCESS_FS_LOCK),
        }
    }

    pub fn root_path(&self) -> String {
        self.root.lock().clone()
    }

    pub fn cwd_path(&self) -> String {
        self.cwd.lock().clone()
    }

    pub fn set_cwd(&self, path: &str) -> Result<(), myos_vfs::Errno> {
        let mut stored = String::new();
        stored
            .try_reserve(path.len())
            .map_err(|_| myos_vfs::Errno::Enomem)?;
        stored.push_str(path);
        *self.cwd.lock() = stored;
        Ok(())
    }

    fn copy_from(&self, other: &Self) -> Result<(), ProcessError> {
        let root = other.root_path();
        let cwd = other.cwd_path();
        let mut stored_root = String::new();
        stored_root
            .try_reserve(root.len())
            .map_err(|_| ProcessError::MetadataOutOfMemory)?;
        stored_root.push_str(&root);
        let mut stored_cwd = String::new();
        stored_cwd
            .try_reserve(cwd.len())
            .map_err(|_| ProcessError::MetadataOutOfMemory)?;
        stored_cwd.push_str(&cwd);
        *self.root.lock() = stored_root;
        *self.cwd.lock() = stored_cwd;
        Ok(())
    }
}

struct ThreadGroup {
    leader: Option<ThreadId>,
    members: Vec<ThreadId>,
}

struct ProcessRelations {
    parent: Option<ProcessId>,
    children: Vec<Arc<Process>>,
    zombie_children: Vec<(Arc<Process>, isize)>,
}

impl ProcessRelations {
    const fn new() -> Self {
        Self {
            parent: None,
            children: Vec::new(),
            zombie_children: Vec::new(),
        }
    }
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
    mm: IrqSpinLock<Arc<UserMm>>,
    files: FileTable,
    signals: SignalState,
    credentials: Credentials,
    fs: FsContext,
    relations: IrqSpinLock<ProcessRelations>,
    child_wait: WaitQueue,
    process_group: AtomicIsize,
    session: AtomicIsize,
    thread_group: IrqSpinLock<ThreadGroup>,
    thread_exit: WaitQueue,
    vfork_pending: AtomicBool,
    vfork_done: Completion,
}

impl Process {
    pub fn create(mm: Box<UserMm>) -> Arc<Self> {
        Self::create_with_mm(Arc::from(mm))
    }

    fn create_with_mm(mm: Arc<UserMm>) -> Arc<Self> {
        let id = ProcessId(allocate_process_id());
        let process = Arc::new(Self {
            id,
            mm: IrqSpinLock::new_with_class(mm, PROCESS_MM_LOCK),
            files: FileTable::new(),
            signals: SignalState::new(),
            credentials: Credentials::bootstrap(),
            fs: FsContext::bootstrap(),
            relations: IrqSpinLock::new_with_class(ProcessRelations::new(), PROCESS_RELATION_LOCK),
            child_wait: WaitQueue::new(),
            process_group: AtomicIsize::new(id.get() as isize),
            session: AtomicIsize::new(id.get() as isize),
            thread_group: IrqSpinLock::new_with_class(
                ThreadGroup::new(),
                PROCESS_THREAD_GROUP_LOCK,
            ),
            thread_exit: WaitQueue::new(),
            vfork_pending: AtomicBool::new(false),
            vfork_done: Completion::new(),
        });
        register_process(&process);
        LIVE_PROCESSES.fetch_add(1, Ordering::AcqRel);
        process
    }

    pub fn fork_child(self: &Arc<Self>, mm: Box<UserMm>) -> Result<Arc<Self>, ProcessError> {
        self.fork_child_with_mm(Arc::from(mm), false)
    }

    /// Create a distinct process that temporarily shares this process's MM.
    ///
    /// This is the address-space half of Linux `CLONE_VM | CLONE_VFORK`.
    /// The caller must wait on `wait_vfork_done` after making the child
    /// runnable; the child releases it after replacing the shared MM in exec,
    /// or on exit.  Keeping distinct Process objects preserves copied fd/fs
    /// state while avoiding an eager copy of a large Cargo/rustc address space.
    pub fn fork_child_shared_mm(
        self: &Arc<Self>,
        vfork_pending: bool,
    ) -> Result<Arc<Self>, ProcessError> {
        self.fork_child_with_mm(self.mm_arc(), vfork_pending)
    }

    fn fork_child_with_mm(
        self: &Arc<Self>,
        mm: Arc<UserMm>,
        vfork_pending: bool,
    ) -> Result<Arc<Self>, ProcessError> {
        let child = Self::create_with_mm(mm);
        child.vfork_pending.store(vfork_pending, Ordering::Release);
        {
            let mut child_files = child.files.table.lock();
            *child_files = self.files.table.lock().fork_clone();
        }
        child.fs.copy_from(&self.fs)?;
        child.signals.set_blocked(self.signals.blocked());
        child.signals.copy_actions_from(&self.signals);
        child
            .process_group
            .store(self.process_group(), Ordering::Release);
        child.session.store(self.session(), Ordering::Release);
        child.set_parent(self.id())?;
        self.add_child(Arc::clone(&child))?;
        Ok(child)
    }

    pub fn wait_vfork_done(&self) {
        if self.vfork_pending.load(Ordering::Acquire) {
            self.vfork_done.wait();
        }
    }

    pub fn complete_vfork(&self) {
        if self.vfork_pending.swap(false, Ordering::AcqRel) {
            self.vfork_done.complete_all();
        }
    }

    pub const fn id(&self) -> ProcessId {
        self.id
    }

    pub fn mm(&self) -> &UserMm {
        let mm = self.mm.lock();
        // SAFETY: Process owns the Arc<UserMm> for its complete lifetime. This
        // borrowed reference is used only by call sites that immediately use it
        // without retaining the reference across a possible exec replacement.
        unsafe { &*Arc::as_ptr(&mm) }
    }

    pub(crate) fn mm_arc(&self) -> Arc<UserMm> {
        Arc::clone(&self.mm.lock())
    }

    fn mm_strong_count(&self) -> usize {
        Arc::strong_count(&self.mm.lock())
    }

    pub fn replace_mm(&self, mm: Box<UserMm>) -> Arc<UserMm> {
        let mut slot = self.mm.lock();
        core::mem::replace(&mut *slot, Arc::from(mm))
    }

    pub fn files(&self) -> &FileTable {
        &self.files
    }

    pub fn signals(&self) -> &SignalState {
        &self.signals
    }

    pub fn parent_id(&self) -> Option<ProcessId> {
        self.relations.lock().parent
    }

    #[allow(dead_code)]
    pub fn set_parent(&self, parent: ProcessId) -> Result<(), ProcessError> {
        let mut relations = self.relations.lock();
        relations.parent = Some(parent);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn add_child(&self, child: Arc<Process>) -> Result<(), ProcessError> {
        let mut relations = self.relations.lock();
        relations
            .children
            .try_reserve(1)
            .map_err(|_| ProcessError::MetadataOutOfMemory)?;
        relations.children.push(child);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn mark_child_zombie(
        &self,
        child: Arc<Process>,
        status: isize,
    ) -> Result<(), ProcessError> {
        {
            let mut relations = self.relations.lock();
            let Some(index) = relations
                .children
                .iter()
                .position(|candidate| candidate.id() == child.id())
            else {
                return Err(ProcessError::ThreadNotFound);
            };
            let child = relations.children.remove(index);
            relations
                .zombie_children
                .try_reserve(1)
                .map_err(|_| ProcessError::MetadataOutOfMemory)?;
            relations.zombie_children.push((child, status));
        }
        self.child_wait.wake_all();
        Ok(())
    }

    pub fn wait_zombie_child(
        &self,
        requested: isize,
    ) -> Result<Option<(Arc<Process>, isize)>, ProcessError> {
        let mut relations = self.relations.lock();
        let Some(index) = relations
            .zombie_children
            .iter()
            .position(|(process, _)| requested == -1 || requested == process.id().get() as isize)
        else {
            return Ok(None);
        };
        Ok(Some(relations.zombie_children.remove(index)))
    }

    pub fn has_child(&self, requested: isize) -> bool {
        let relations = self.relations.lock();
        relations
            .children
            .iter()
            .any(|process| requested == -1 || requested == process.id().get() as isize)
            || relations
                .zombie_children
                .iter()
                .any(|(process, _)| requested == -1 || requested == process.id().get() as isize)
    }

    pub fn has_zombie_child(&self, requested: isize) -> bool {
        let relations = self.relations.lock();
        relations
            .zombie_children
            .iter()
            .any(|(process, _)| requested == -1 || requested == process.id().get() as isize)
    }

    pub const fn child_wait_queue(&self) -> &WaitQueue {
        &self.child_wait
    }

    pub fn process_group(&self) -> isize {
        self.process_group.load(Ordering::Acquire)
    }

    pub fn set_process_group(&self, pgid: isize) {
        self.process_group.store(pgid, Ordering::Release);
    }

    pub fn session(&self) -> isize {
        self.session.load(Ordering::Acquire)
    }

    pub fn setsid(&self) -> isize {
        let sid = self.id.get() as isize;
        self.session.store(sid, Ordering::Release);
        self.process_group.store(sid, Ordering::Release);
        sid
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
            user_sp: AtomicUsize::new(user_stack.end().get()),
            trap_frame: IrqSpinLock::new_with_class(None, THREAD_TRAP_FRAME_LOCK),
            tls: AtomicUsize::new(0),
            clear_child_tid: AtomicUsize::new(0),
            robust_list_head: AtomicUsize::new(0),
            blocked_signals: AtomicU64::new(0),
            sigsuspend_saved_mask: AtomicU64::new(NO_SIGSUSPEND_SAVED_MASK),
            scheduler_task: AtomicUsize::new(UNBOUND_SCHEDULER_TASK),
            visited_cpus: AtomicUsize::new(0),
            schedule_count: AtomicUsize::new(0),
            lifecycle: AtomicU8::new(THREAD_READY),
            exit_status: AtomicIsize::new(0),
            forced_exit_status: AtomicIsize::new(NO_FORCED_EXIT),
            exited: Completion::new(),
        });
        LIVE_THREADS.fetch_add(1, Ordering::AcqRel);
        Ok(thread)
    }

    /// Creates a non-leader member of this process's thread group.
    ///
    /// Keeping every CLONE_THREAD task on the same Process object is what
    /// provides the Linux CLONE_VM/CLONE_FILES/CLONE_FS/CLONE_SIGHAND sharing
    /// semantics. In particular, one pthread exiting must not close a copied
    /// descriptor table and accidentally publish EOF on a process-wide pipe.
    pub fn create_thread(
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
        if group.leader.is_none() {
            return Err(ProcessError::ThreadNotReady);
        }
        group
            .members
            .try_reserve(1)
            .map_err(|_| ProcessError::MetadataOutOfMemory)?;

        let id = ThreadId(allocate_process_id());
        group.members.push(id);
        drop(group);

        let thread = Arc::new(Thread {
            id,
            process: Arc::clone(self),
            user_pc: AtomicUsize::new(entry.get()),
            user_stack,
            user_sp: AtomicUsize::new(user_stack.end().get()),
            trap_frame: IrqSpinLock::new_with_class(None, THREAD_TRAP_FRAME_LOCK),
            tls: AtomicUsize::new(0),
            clear_child_tid: AtomicUsize::new(0),
            robust_list_head: AtomicUsize::new(0),
            blocked_signals: AtomicU64::new(0),
            sigsuspend_saved_mask: AtomicU64::new(NO_SIGSUSPEND_SAVED_MASK),
            scheduler_task: AtomicUsize::new(UNBOUND_SCHEDULER_TASK),
            visited_cpus: AtomicUsize::new(0),
            schedule_count: AtomicUsize::new(0),
            lifecycle: AtomicU8::new(THREAD_READY),
            exit_status: AtomicIsize::new(0),
            forced_exit_status: AtomicIsize::new(NO_FORCED_EXIT),
            exited: Completion::new(),
        });
        LIVE_THREADS.fetch_add(1, Ordering::AcqRel);
        Ok(thread)
    }

    /// Consumes the final unique process owner.
    ///
    /// Resource teardown is RAII-backed by `Drop`.  This is important for
    /// orphaned children: a parent may exit without waiting for every zombie,
    /// and the last child owner must still unregister the process and release
    /// its address space instead of depending on a successful `wait4`.
    pub fn destroy(self) -> Result<(), UserMmRuntimeError> {
        {
            let group = self.thread_group.lock();
            assert!(
                group.members.is_empty() && group.leader.is_none(),
                "M9-B attempted to destroy a Process with live thread-group members",
            );
        }
        Ok(())
    }

    pub fn wait_until_single_thread(&self) {
        self.thread_exit.wait_until(|| self.thread_count() <= 1);
    }

    fn detach_thread(&self, id: ThreadId) -> Result<bool, ProcessError> {
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
        let empty = group.members.is_empty();
        drop(group);
        if empty {
            // Multiple members may enter Thread::exit before either retired
            // Thread is dropped, so a pre-detach thread_count check can miss
            // the 2 -> 0 transition. Final detach is the unique process-exit
            // point and must close inherited pipe/jobserver descriptors before
            // the parent observes the zombie.
            let _ = self.files.close_all();
            crate::user::fcntl_release_process_locks(self.id.get());
        }
        self.thread_exit.wake_all();
        Ok(empty)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let group = self.thread_group.lock();
        assert!(
            group.members.is_empty() && group.leader.is_none(),
            "dropping a Process with live thread-group members",
        );
        drop(group);

        unregister_process(self.id);
        LIVE_PROCESSES.fetch_sub(1, Ordering::AcqRel);
        // `mm` is dropped after this method returns. `UserMm::drop` performs
        // the fallible page-table teardown and turns an impossible teardown
        // failure into a kernel panic, matching the previous explicit path.
    }
}

fn register_process(process: &Arc<Process>) {
    let mut registry = PROCESS_REGISTRY.lock();
    let map = registry.get_or_insert_with(BTreeMap::new);
    map.insert(process.id(), Arc::downgrade(process));
}

fn unregister_process(pid: ProcessId) {
    let mut registry = PROCESS_REGISTRY.lock();
    if let Some(map) = registry.as_mut() {
        map.remove(&pid);
    }
}

pub fn lookup_process(pid: ProcessId) -> Option<Arc<Process>> {
    PROCESS_REGISTRY
        .lock()
        .as_ref()
        .and_then(|registry| registry.get(&pid))
        .and_then(alloc::sync::Weak::upgrade)
}

/// Call `f` once for every live process.  Collects a snapshot of
/// `Arc<Process>` references first, then invokes `f` without holding
/// the process registry lock.  This is essential because `f` may
/// perform arbitrary work (signal delivery, lock acquisition, etc.).
pub fn for_each_process(mut f: impl FnMut(&Arc<Process>)) {
    let snapshot: alloc::vec::Vec<Arc<Process>> = {
        let registry = PROCESS_REGISTRY.lock();
        match registry.as_ref() {
            Some(map) => map
                .values()
                .filter_map(alloc::sync::Weak::upgrade)
                .collect(),
            None => alloc::vec::Vec::new(),
        }
    };

    for process in snapshot.iter() {
        f(process);
    }
}

pub struct Thread {
    id: ThreadId,
    process: Arc<Process>,
    user_pc: AtomicUsize,
    user_stack: VirtRange,
    user_sp: AtomicUsize,
    trap_frame: IrqSpinLock<Option<crate::arch::trap::TrapFrame>>,
    tls: AtomicUsize,
    /// CLONE_CHILD_CLEARTID: user-space address where 0 is written
    /// and a futex wake is performed when this thread exits.
    clear_child_tid: AtomicUsize,
    robust_list_head: AtomicUsize,
    blocked_signals: AtomicU64,
    sigsuspend_saved_mask: AtomicU64,
    scheduler_task: AtomicUsize,
    visited_cpus: AtomicUsize,
    schedule_count: AtomicUsize,
    lifecycle: AtomicU8,
    exit_status: AtomicIsize,
    forced_exit_status: AtomicIsize,
    exited: Completion,
}

impl Thread {
    pub const fn id(&self) -> ThreadId {
        self.id
    }

    pub fn process(&self) -> &Process {
        self.process.as_ref()
    }

    pub fn process_arc(&self) -> Arc<Process> {
        Arc::clone(&self.process)
    }

    pub fn entry(&self) -> VirtAddr {
        VirtAddr::new(self.user_pc.load(Ordering::Acquire))
    }

    pub const fn user_stack(&self) -> VirtRange {
        self.user_stack
    }

    pub fn user_stack_pointer(&self) -> VirtAddr {
        VirtAddr::new(self.user_sp.load(Ordering::Acquire))
    }

    #[allow(dead_code)]
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

    pub fn prepare_stack_pointer(&self, stack_pointer: VirtAddr) -> Result<(), ProcessError> {
        if self.lifecycle.load(Ordering::Acquire) != THREAD_READY {
            return Err(ProcessError::ThreadNotReady);
        }
        if stack_pointer.get() & 0xf != 0 || !self.user_stack.contains(stack_pointer) {
            return Err(ProcessError::InvalidUserContext);
        }
        self.user_sp.store(stack_pointer.get(), Ordering::Release);
        Ok(())
    }

    pub fn exec_replace_context(
        &self,
        entry: VirtAddr,
        user_stack: VirtRange,
        stack_pointer: VirtAddr,
    ) -> Result<(), ProcessError> {
        if self.lifecycle.load(Ordering::Acquire) != THREAD_RUNNING {
            return Err(ProcessError::ThreadNotReady);
        }
        let user_range = crate::arch::memory::layout::USER_RANGE;
        if user_stack != self.user_stack
            || !user_range.contains(entry)
            || user_stack.is_empty()
            || !user_range.contains_range(user_stack)
            || stack_pointer.get() & 0xf != 0
            || !(stack_pointer == user_stack.end() || user_stack.contains(stack_pointer))
        {
            return Err(ProcessError::InvalidUserContext);
        }
        self.user_pc.store(entry.get(), Ordering::Release);
        self.user_sp.store(stack_pointer.get(), Ordering::Release);
        *self.trap_frame.lock() = None;
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
        self.process.complete_vfork();
        self.exited.complete_all();
        Ok(())
    }

    pub(crate) fn request_forced_exit(&self, status: isize) {
        let _ = self.forced_exit_status.compare_exchange(
            NO_FORCED_EXIT,
            status,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub(crate) fn forced_exit_status(&self) -> Option<isize> {
        let status = self.forced_exit_status.load(Ordering::Acquire);
        (status != NO_FORCED_EXIT).then_some(status)
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

    /// Install rt_sigsuspend's temporary mask and retain the mask that must
    /// be restored after a caught handler completes.
    pub fn begin_sigsuspend(&self, temporary_mask: u64) {
        let old_mask = self.blocked_signals();
        let previous = self
            .sigsuspend_saved_mask
            .swap(old_mask, Ordering::AcqRel);
        debug_assert_eq!(
            previous,
            NO_SIGSUSPEND_SAVED_MASK,
            "nested rt_sigsuspend restore state",
        );
        self.set_blocked_signals(temporary_mask);
    }

    /// Consume the mask saved by rt_sigsuspend for a user signal frame.
    pub fn take_sigsuspend_saved_mask(&self) -> Option<u64> {
        let saved = self
            .sigsuspend_saved_mask
            .swap(NO_SIGSUSPEND_SAVED_MASK, Ordering::AcqRel);
        (saved != NO_SIGSUSPEND_SAVED_MASK).then_some(saved)
    }

    /// A signal action that creates no user frame must still end the
    /// temporary rt_sigsuspend mask interval.
    pub fn restore_sigsuspend_mask_without_handler(&self) {
        if let Some(saved) = self.take_sigsuspend_saved_mask() {
            self.set_blocked_signals(saved);
        }
    }

    #[allow(dead_code)]
    pub fn set_tls(&self, value: usize) {
        self.tls.store(value, Ordering::Release);
    }

    /// CLONE_SETTLS: set the thread's TLS register value (used as `tp`).
    /// This is an alias for `set_tls` with a CLONE-aware name.
    pub fn set_tls_pointer(&self, value: usize) {
        self.tls.store(value, Ordering::Release);
    }

    /// CLONE_CHILD_CLEARTID: save the user-space address where 0 will be
    /// written and futex-woken when this thread exits.
    pub fn set_clear_child_tid(&self, address: usize) {
        self.clear_child_tid.store(address, Ordering::Release);
    }

    /// Return the clear_child_tid address, or 0 if none was set.
    pub fn clear_child_tid_address(&self) -> usize {
        self.clear_child_tid.load(Ordering::Acquire)
    }

    pub fn set_robust_list_head(&self, address: usize) {
        self.robust_list_head.store(address, Ordering::Release);
    }

    pub fn robust_list_head(&self) -> usize {
        self.robust_list_head.load(Ordering::Acquire)
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
        let process_became_empty = self
            .process
            .detach_thread(self.id)
            .expect("M9-B Thread disappeared from its process thread group");
        if process_became_empty
            && let Some(parent) = self
                .process
                .parent_id()
                .and_then(crate::process::lookup_process)
        {
            let status = self
                .exit_status()
                .expect("Thread dropped before publishing an exit status");
            if parent
                .mark_child_zombie(Arc::clone(&self.process), status)
                .is_ok()
            {
                let _ = parent.signals().add_pending(crate::signal::SIGCHLD);
            }
        }
        LIVE_THREADS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn assert_initial_pair(process: &Arc<Process>, thread: &Arc<Thread>) {
    assert_eq!(thread.process().id(), process.id());
    assert_eq!(thread.id().get(), process.id().get());
    assert_eq!(process.thread_count(), 1);
    assert_eq!(Arc::strong_count(process), 2);
    assert_eq!(process.mm_strong_count(), 1);
    assert_eq!(process.files().open_count(), 3);
    assert_eq!(process.signals().pending(), 0);
    assert_eq!(process.credentials().real_uid(), 0);
    assert_eq!(process.credentials().effective_uid(), 0);
    assert_eq!(process.credentials().real_gid(), 0);
    assert_eq!(process.credentials().effective_gid(), 0);
    assert_eq!(process.fs().root_path(), "/");
    assert_eq!(process.fs().cwd_path(), "/");
    assert_eq!(thread.tls(), 0);
    assert_eq!(thread.blocked_signals(), 0);
    assert!(thread.exit_status().is_none());
    let stack_pointer = thread.user_stack_pointer();
    assert_eq!(stack_pointer.get() & 0xf, 0);
    assert!(
        stack_pointer == thread.user_stack().end() || thread.user_stack().contains(stack_pointer),
        "initial user stack pointer is outside the declared stack VMA",
    );
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

pub(crate) fn live_process_count() -> usize {
    LIVE_PROCESSES.load(Ordering::Acquire)
}

pub(crate) fn live_thread_count() -> usize {
    LIVE_THREADS.load(Ordering::Acquire)
}

fn allocate_process_id() -> usize {
    NEXT_PROCESS_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("M9-A exhausted the process-ID namespace")
}
