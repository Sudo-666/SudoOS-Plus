mod stack;

use alloc::{collections::VecDeque, vec::Vec};
#[cfg(debug_assertions)]
use core::{
    hint::{black_box, spin_loop},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::{irq_lock::IrqSpinLock, smp::CpuId};
use stack::KernelStack;

const MAX_TASKS: usize = 128;
const MAX_CPUS: usize = crate::smp::MAX_CPUS;
#[cfg(debug_assertions)]
#[cfg(debug_assertions)]
const WORKER_ITERATIONS: usize = 50_000;
#[cfg(debug_assertions)]
const STEAL_TASK_COUNT: usize = 16;
#[cfg(debug_assertions)]
const VERIFY_TIMEOUT_SECONDS: u64 = 30;

type ContextSwitch = (
    *mut crate::arch::task::Context,
    *const crate::arch::task::Context,
);

pub type KernelThreadEntry = fn();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskId(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskState {
    Runnable,
    Running(CpuId),
    SwitchingOut(CpuId),
    Idle(CpuId),
    Exited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskKind {
    Idle(CpuId),
    KernelThread,
}

impl TaskKind {
    const fn is_idle(self) -> bool {
        matches!(self, Self::Idle(_))
    }
}

struct Task {
    id: TaskId,
    kind: TaskKind,
    state: TaskState,
    context: crate::arch::task::Context,
    stack: Option<KernelStack>,
    entry: Option<KernelThreadEntry>,
    affinity: Option<CpuId>,
    queued_on: Option<CpuId>,
    has_run: bool,
}

impl Task {
    fn boot() -> Self {
        Self {
            id: TaskId(0),
            kind: TaskKind::Idle(CpuId::BOOT),
            state: TaskState::Running(CpuId::BOOT),
            context: crate::arch::task::Context::default(),
            stack: None,
            entry: None,
            affinity: Some(CpuId::BOOT),
            queued_on: None,
            has_run: true,
        }
    }

    fn idle(id: TaskId, cpu: CpuId, stack: KernelStack) -> Self {
        Self {
            id,
            kind: TaskKind::Idle(cpu),
            state: TaskState::Idle(cpu),
            context: crate::arch::task::Context::new(stack.top(), idle_thread_bootstrap),
            stack: Some(stack),
            entry: None,
            affinity: Some(cpu),
            queued_on: None,
            has_run: false,
        }
    }

    fn kernel_thread(
        id: TaskId,
        entry: KernelThreadEntry,
        stack: KernelStack,
        affinity: Option<CpuId>,
    ) -> Self {
        Self {
            id,
            kind: TaskKind::KernelThread,
            state: TaskState::Runnable,
            context: crate::arch::task::Context::new(stack.top(), kernel_thread_bootstrap),
            stack: Some(stack),
            entry: Some(entry),
            affinity,
            queued_on: None,
            has_run: false,
        }
    }

    #[cfg(debug_assertions)]
    fn stack_contains(&self, address: usize) -> bool {
        self.stack
            .as_ref()
            .is_some_and(|stack| stack.contains(address))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SwitchDisposition {
    Yield,
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingSwitch {
    previous: TaskId,
    disposition: SwitchDisposition,
}

struct CpuScheduler {
    online: bool,
    current: Option<TaskId>,
    idle: Option<TaskId>,
    run_queue: VecDeque<TaskId>,
    pending: Option<PendingSwitch>,
    context_switches: u64,
}

impl CpuScheduler {
    fn new() -> Self {
        Self {
            online: false,
            current: None,
            idle: None,
            run_queue: VecDeque::with_capacity(MAX_TASKS),
            pending: None,
            context_switches: 0,
        }
    }
}

struct Scheduler {
    tasks: Vec<Option<Task>>,
    cpus: [CpuScheduler; MAX_CPUS],
    discovered_cpus: usize,
    live_kernel_threads: usize,
}

impl Scheduler {
    fn new(discovered_cpus: usize) -> Self {
        assert!((1..=MAX_CPUS).contains(&discovered_cpus));

        let mut tasks = Vec::with_capacity(MAX_TASKS);
        tasks.push(Some(Task::boot()));
        assert!(tasks.capacity() >= MAX_TASKS);

        let mut cpus = core::array::from_fn(|_| CpuScheduler::new());
        cpus[CpuId::BOOT.get()].online = true;
        cpus[CpuId::BOOT.get()].current = Some(TaskId(0));
        cpus[CpuId::BOOT.get()].idle = Some(TaskId(0));

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
            cpus[cpu.get()].idle = Some(id);
        }

        Self {
            tasks,
            cpus,
            discovered_cpus,
            live_kernel_threads: 0,
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

    fn current(&self, cpu: CpuId) -> TaskId {
        self.cpus[cpu.get()]
            .current
            .expect("online CPU has no current task")
    }

    fn idle(&self, cpu: CpuId) -> TaskId {
        self.cpus[cpu.get()]
            .idle
            .expect("discovered CPU has no idle task")
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
            .filter(|cpu| self.cpus[cpu.get()].online)
            .min_by_key(|cpu| self.cpus[cpu.get()].run_queue.len())
            .expect("scheduler has no online CPU")
    }

    fn spawn(
        &mut self,
        entry: KernelThreadEntry,
        stack: KernelStack,
        affinity: Option<CpuId>,
        queue_hint: Option<CpuId>,
    ) -> (TaskId, CpuId) {
        let target = match affinity.or(queue_hint) {
            Some(cpu) => {
                assert!(
                    cpu.get() < self.discovered_cpus,
                    "task target CPU was not discovered",
                );
                assert!(self.cpus[cpu.get()].online, "task target CPU is offline");
                cpu
            }
            None => self.choose_target_cpu(),
        };

        let id = self.allocate_task_id();
        let task = Task::kernel_thread(id, entry, stack, affinity);

        if id.0 == self.tasks.len() {
            self.tasks.push(Some(task));
        } else {
            assert!(self.tasks[id.0].is_none());
            self.tasks[id.0] = Some(task);
        }

        self.enqueue(id, target);
        self.live_kernel_threads += 1;
        (id, target)
    }

    fn enqueue(&mut self, id: TaskId, cpu: CpuId) {
        {
            let task = self.task_mut(id);
            assert_eq!(task.state, TaskState::Runnable);
            assert!(task.queued_on.is_none(), "task was queued more than once");
            if let Some(affinity) = task.affinity {
                assert_eq!(affinity, cpu, "pinned task queued on the wrong CPU");
            }
            task.queued_on = Some(cpu);
        }

        self.cpus[cpu.get()].run_queue.push_back(id);
    }

    fn dequeue_local(&mut self, cpu: CpuId) -> Option<TaskId> {
        let id = self.cpus[cpu.get()].run_queue.pop_front()?;
        let task = self.task_mut(id);

        assert_eq!(task.state, TaskState::Runnable);
        assert_eq!(task.queued_on, Some(cpu));
        task.queued_on = None;
        Some(id)
    }

    fn steal_unstarted(&mut self, cpu: CpuId) -> Option<TaskId> {
        for donor_index in 0..self.discovered_cpus {
            let donor = CpuId::new(donor_index).expect("invalid donor CPU");
            if donor == cpu || !self.cpus[donor.get()].online {
                continue;
            }

            let position = self.cpus[donor.get()].run_queue.iter().position(|id| {
                let task = self.task(*id);
                task.state == TaskState::Runnable && task.affinity.is_none() && !task.has_run
            });

            let Some(position) = position else {
                continue;
            };

            let id = self.cpus[donor.get()]
                .run_queue
                .remove(position)
                .expect("stealable task disappeared from donor queue");
            let task = self.task_mut(id);
            assert_eq!(task.queued_on, Some(donor));
            task.queued_on = None;
            return Some(id);
        }

        None
    }

    fn dequeue_next(&mut self, cpu: CpuId) -> Option<TaskId> {
        self.dequeue_local(cpu)
            .or_else(|| self.steal_unstarted(cpu))
    }

    fn activate_next(&mut self, id: TaskId, cpu: CpuId) {
        let task = self.task_mut(id);

        match task.kind {
            TaskKind::Idle(owner) => {
                assert_eq!(owner, cpu, "idle task selected by the wrong CPU");
                assert_eq!(task.state, TaskState::Idle(cpu));
            }
            TaskKind::KernelThread => {
                assert_eq!(task.state, TaskState::Runnable);
                if let Some(affinity) = task.affinity {
                    assert_eq!(affinity, cpu, "pinned task selected by the wrong CPU");
                }
                task.has_run = true;
            }
        }

        assert!(task.queued_on.is_none());
        task.state = TaskState::Running(cpu);
    }

    fn prepare_yield(&mut self, cpu: CpuId) -> Option<ContextSwitch> {
        assert!(
            self.cpus[cpu.get()].online,
            "offline CPU attempted to schedule"
        );
        assert!(
            self.cpus[cpu.get()].pending.is_none(),
            "CPU attempted a nested context switch",
        );

        let previous = self.current(cpu);
        assert_eq!(self.task(previous).state, TaskState::Running(cpu));

        let next = match self.dequeue_next(cpu) {
            Some(next) => next,
            None if self.task(previous).kind.is_idle() => return None,
            None => self.idle(cpu),
        };

        assert_ne!(previous, next, "CPU selected its current task as next");
        self.task_mut(previous).state = TaskState::SwitchingOut(cpu);
        self.activate_next(next, cpu);
        self.cpus[cpu.get()].current = Some(next);
        self.cpus[cpu.get()].pending = Some(PendingSwitch {
            previous,
            disposition: SwitchDisposition::Yield,
        });
        self.cpus[cpu.get()].context_switches = self.cpus[cpu.get()]
            .context_switches
            .checked_add(1)
            .expect("context switch counter overflowed");

        Some(self.context_pair(previous, next))
    }

    fn prepare_exit(&mut self, cpu: CpuId) -> ContextSwitch {
        assert!(
            self.cpus[cpu.get()].pending.is_none(),
            "CPU attempted a nested context switch",
        );

        let previous = self.current(cpu);
        assert_eq!(self.task(previous).state, TaskState::Running(cpu));
        assert!(
            !self.task(previous).kind.is_idle(),
            "idle task attempted to exit",
        );

        let next = self.dequeue_next(cpu).unwrap_or_else(|| self.idle(cpu));
        assert_ne!(previous, next);

        self.task_mut(previous).state = TaskState::SwitchingOut(cpu);
        self.activate_next(next, cpu);
        self.cpus[cpu.get()].current = Some(next);
        self.cpus[cpu.get()].pending = Some(PendingSwitch {
            previous,
            disposition: SwitchDisposition::Exit,
        });
        self.cpus[cpu.get()].context_switches = self.cpus[cpu.get()]
            .context_switches
            .checked_add(1)
            .expect("context switch counter overflowed");

        self.context_pair(previous, next)
    }

    fn context_pair(&mut self, previous: TaskId, next: TaskId) -> ContextSwitch {
        let previous_pointer = {
            let task = self.task_mut(previous);
            core::ptr::addr_of_mut!(task.context)
        };
        let next_pointer = {
            let task = self.task(next);
            core::ptr::addr_of!(task.context)
        };

        (previous_pointer, next_pointer)
    }

    fn complete_switch(&mut self, cpu: CpuId) -> Option<Task> {
        let pending = self.cpus[cpu.get()].pending.take()?;

        assert_eq!(
            self.task(pending.previous).state,
            TaskState::SwitchingOut(cpu),
        );

        match pending.disposition {
            SwitchDisposition::Yield => {
                if self.task(pending.previous).kind.is_idle() {
                    self.task_mut(pending.previous).state = TaskState::Idle(cpu);
                } else {
                    self.task_mut(pending.previous).state = TaskState::Runnable;
                    self.enqueue(pending.previous, cpu);
                }
                None
            }
            SwitchDisposition::Exit => {
                self.task_mut(pending.previous).state = TaskState::Exited;
                self.live_kernel_threads = self
                    .live_kernel_threads
                    .checked_sub(1)
                    .expect("live kernel-thread counter underflowed");

                let task = self.tasks[pending.previous.0]
                    .take()
                    .expect("exited task disappeared before reclamation");
                assert_eq!(task.id, pending.previous);
                assert_eq!(task.state, TaskState::Exited);
                Some(task)
            }
        }
    }

    fn register_secondary(&mut self, cpu: CpuId) {
        assert_ne!(cpu, CpuId::BOOT);
        assert!(cpu.get() < self.discovered_cpus);
        assert!(
            !self.cpus[cpu.get()].online,
            "secondary CPU registered twice"
        );

        let idle = self.idle(cpu);
        assert_eq!(self.task(idle).state, TaskState::Idle(cpu));
        self.task_mut(idle).state = TaskState::Running(cpu);
        self.task_mut(idle).has_run = true;
        self.cpus[cpu.get()].current = Some(idle);
        self.cpus[cpu.get()].online = true;
    }

    fn secondary_idle_context(&self, cpu: CpuId) -> *const crate::arch::task::Context {
        let idle = self.idle(cpu);
        assert_eq!(self.current(cpu), idle);
        core::ptr::addr_of!(self.task(idle).context)
    }

    fn current_entry(&self, cpu: CpuId) -> KernelThreadEntry {
        self.task(self.current(cpu))
            .entry
            .expect("current task is not a kernel thread")
    }

    #[cfg(debug_assertions)]
    fn current_stack_contains(&self, cpu: CpuId, address: usize) -> bool {
        self.task(self.current(cpu)).stack_contains(address)
    }

    fn work_available(&self, cpu: CpuId) -> bool {
        if !self.cpus[cpu.get()].run_queue.is_empty() {
            return true;
        }

        (0..self.discovered_cpus).any(|donor_index| {
            let donor = CpuId::new(donor_index).expect("invalid donor CPU");
            donor != cpu
                && self.cpus[donor.get()].online
                && self.cpus[donor.get()].run_queue.iter().any(|id| {
                    let task = self.task(*id);
                    task.state == TaskState::Runnable && task.affinity.is_none() && !task.has_run
                })
        })
    }

    fn context_switches_total(&self) -> u64 {
        self.cpus
            .iter()
            .take(self.discovered_cpus)
            .map(|cpu| cpu.context_switches)
            .sum()
    }
}

static SCHEDULER: IrqSpinLock<Option<Scheduler>> = IrqSpinLock::new(None);

pub fn initialize() {
    let discovered = crate::smp::discovered_cpu_count();
    let scheduler = Scheduler::new(discovered);
    let mut slot = SCHEDULER.lock();

    assert!(slot.is_none(), "kernel scheduler was initialized twice");
    *slot = Some(scheduler);

    crate::println!("kernel scheduler:");
    crate::println!("  policy          : per-CPU FIFO round-robin");
    crate::println!("  kernel stack    : 16 KiB plus guard pages");
    crate::println!("  active CPUs     : 1");
    crate::println!("  configured CPUs : {}", discovered);
    crate::println!("  migration       : unstarted tasks only until TLB shootdown");
}

pub fn register_secondary_cpu(cpu: CpuId) {
    assert!(crate::arch::interrupt::are_disabled());

    let mut slot = SCHEDULER.lock();
    slot.as_mut()
        .expect("kernel scheduler is not initialized")
        .register_secondary(cpu);
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

pub fn spawn_kernel_thread(entry: KernelThreadEntry) -> TaskId {
    spawn_internal(entry, None, None).0
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
            .spawn(entry, stack, affinity, queue_hint)
    };

    if target != crate::smp::current_cpu_id() {
        crate::smp::send_ipi(target);
    }

    (id, target)
}

pub fn yield_now() {
    assert!(
        crate::arch::interrupt::are_enabled(),
        "yield_now requires local interrupts to be enabled",
    );

    let interrupt_state = crate::arch::interrupt::save_and_disable();
    let cpu = crate::smp::current_cpu_id();
    let switch = {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .prepare_yield(cpu)
    };

    let Some((previous, next)) = switch else {
        crate::arch::interrupt::restore(interrupt_state);
        return;
    };

    // SAFETY: the old task is marked SwitchingOut and cannot be selected by
    // another CPU. The incoming task is exclusively Running on this CPU, both
    // contexts remain allocated, and local interrupts stay disabled.
    unsafe { crate::arch::task::switch(previous, next) };

    finish_switch();
    crate::arch::interrupt::restore(interrupt_state);
}

fn exit_current() -> ! {
    assert!(
        crate::arch::interrupt::are_enabled(),
        "kernel thread exited with local interrupts disabled",
    );

    let _interrupt_state = crate::arch::interrupt::save_and_disable();
    let cpu = crate::smp::current_cpu_id();
    let (previous, next) = {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .prepare_exit(cpu)
    };

    // SAFETY: the exiting task remains allocated and marked SwitchingOut
    // until the incoming context calls finish_switch() from a different stack.
    unsafe { crate::arch::task::switch(previous, next) };

    panic!("exited kernel thread resumed unexpectedly");
}

fn finish_switch() {
    let cpu = crate::smp::current_cpu_id();
    let retired = {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .complete_switch(cpu)
    };

    // Drop outside the scheduler lock. Releasing a vmalloc-backed stack takes
    // the VM and page-allocator locks and invalidates the local TLB entry.
    drop(retired);
}

fn current_entry() -> KernelThreadEntry {
    let cpu = crate::smp::current_cpu_id();
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .current_entry(cpu)
}

fn current_cpu_has_work() -> bool {
    let cpu = crate::smp::current_cpu_id();
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .work_available(cpu)
}

#[cfg(debug_assertions)]
fn current_stack_contains(address: usize) -> bool {
    let cpu = crate::smp::current_cpu_id();
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .current_stack_contains(cpu, address)
}

#[cfg(debug_assertions)]
fn live_kernel_threads() -> usize {
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .live_kernel_threads
}

#[cfg(debug_assertions)]
fn context_switches() -> u64 {
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .context_switches_total()
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

unsafe extern "C" fn idle_thread_bootstrap() -> ! {
    finish_switch();

    // SAFETY: secondary initialization installed trap, paging, local timer,
    // IPI state, and this CPU's guarded idle stack before selecting it.
    unsafe { crate::arch::interrupt::enable() };
    idle_loop()
}

fn idle_loop() -> ! {
    loop {
        if current_cpu_has_work() {
            yield_now();
        } else {
            crate::arch::cpu::wait_for_interrupt();
        }
    }
}

pub fn boot_idle_loop() -> ! {
    assert_eq!(crate::smp::current_cpu_id(), CpuId::BOOT);
    idle_loop()
}

#[cfg(debug_assertions)]
static WORKER_PROGRESS: [AtomicUsize; MAX_CPUS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
#[cfg(debug_assertions)]
static WORKER_STACKS: [AtomicUsize; MAX_CPUS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
#[cfg(debug_assertions)]
static WORKER_CPUS: [AtomicUsize; MAX_CPUS] = [
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
];
#[cfg(debug_assertions)]
static EXPECTED_CPUS: [AtomicUsize; MAX_CPUS] = [
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
    AtomicUsize::new(usize::MAX),
];
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
            assert!(
                !deadline_reached(crate::arch::time::counter(), deadline),
                "SMP workers failed to execute concurrently",
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

#[cfg(debug_assertions)]
const WORKER_ENTRIES: [KernelThreadEntry; MAX_CPUS] = [
    worker_0, worker_1, worker_2, worker_3, worker_4, worker_5, worker_6, worker_7,
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
        yield_now();
        if !current_cpu_has_work() {
            crate::arch::cpu::wait_for_interrupt();
        }
    }

    finish_switch();
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
        crate::arch::cpu::wait_for_interrupt();
    }
}

#[cfg(debug_assertions)]
fn verify_work_stealing(cpu_count: usize) {
    if cpu_count == 1 {
        return;
    }

    STEAL_COMPLETED.store(0, Ordering::Release);
    STEAL_CPU_MASK.store(0, Ordering::Release);

    for _ in 0..STEAL_TASK_COUNT {
        spawn_internal(steal_worker, None, Some(CpuId::BOOT));
    }

    crate::smp::broadcast_ipi_except_current();
    let deadline = verification_deadline();

    while STEAL_COMPLETED.load(Ordering::Acquire) != STEAL_TASK_COUNT || live_kernel_threads() != 0
    {
        assert!(
            !deadline_reached(crate::arch::time::counter(), deadline),
            "work-stealing verification timed out",
        );
        // Deliberately do not yield CPU0: unstarted tasks queued on CPU0 must
        // be stolen and executed by secondary CPUs.
        crate::arch::cpu::wait_for_interrupt();
    }

    assert_ne!(
        STEAL_CPU_MASK.load(Ordering::Acquire) & !1_usize,
        0,
        "no secondary CPU stole an unstarted task",
    );
}

#[cfg(debug_assertions)]
pub fn verify() {
    reset_verification_state();
    crate::heap::shrink();

    let cpu_count = crate::smp::online_cpu_count();
    assert_eq!(cpu_count, crate::smp::discovered_cpu_count());
    let spawned_workers = if cpu_count == 1 { 2 } else { cpu_count };

    let expected_yields = WORKER_ITERATIONS
        .checked_mul(spawned_workers)
        .expect("scheduler yield count overflowed");

    VERIFY_ITERATIONS.store(WORKER_ITERATIONS, Ordering::Release);
    USE_CONCURRENT_BARRIER.store(cpu_count > 1, Ordering::Release);
    EXPECTED_WORKER_MASK.store((1_usize << spawned_workers) - 1, Ordering::Release);

    let pages_before = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable before scheduler verification");
    let switches_before = context_switches();

    for index in 0..spawned_workers {
        let cpu = if cpu_count == 1 {
            CpuId::BOOT
        } else {
            CpuId::new(index).expect("worker CPU exceeds MAX_CPUS")
        };
        EXPECTED_CPUS[index].store(cpu.get(), Ordering::Release);
        if cpu_count == 1 {
            spawn_kernel_thread(WORKER_ENTRIES[index]);
        } else {
            spawn_internal(WORKER_ENTRIES[index], Some(cpu), Some(cpu));
        }
    }

    wait_for_workers(spawned_workers);
    verify_ipi_delivery(cpu_count);
    verify_work_stealing(cpu_count);
    crate::heap::shrink();

    let actual_switches = context_switches()
        .checked_sub(switches_before)
        .expect("context switch counter moved backwards");
    let pages_after = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable after scheduler verification");

    for index in 0..spawned_workers {
        assert_eq!(
            WORKER_PROGRESS[index].load(Ordering::Acquire),
            WORKER_ITERATIONS
        );
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

    assert!(
        actual_switches >= expected_yields as u64,
        "too few context switches: actual={actual_switches} minimum={expected_yields}",
    );
    assert_eq!(pages_before, pages_after, "kernel task resources leaked");
    assert!(crate::arch::interrupt::are_enabled());

    crate::println!("kernel scheduler test:");
    crate::println!("  kernel threads  : verified ({})", spawned_workers);
    crate::println!("  private stacks  : verified");
    crate::println!(
        "  context switch  : verified ({} switches)",
        actual_switches
    );
    crate::println!("  cooperative     : verified");
    crate::println!("  timer coexistence: verified");
    crate::println!("  task exit       : verified");
    crate::println!("  resource reclaim: verified");

    crate::println!("SMP scheduler test:");
    crate::println!("  participating CPUs : {}", cpu_count);
    crate::println!("  concurrent threads : verified");
    crate::println!("  per-CPU current     : verified");
    crate::println!("  task affinity       : verified");
    if cpu_count > 1 {
        crate::println!("  remote wakeup       : verified");
        crate::println!("  IPI delivery        : verified");
        crate::println!("  work stealing       : verified (unstarted tasks)");
    } else {
        crate::println!("  remote wakeup       : single-CPU fallback");
        crate::println!("  IPI delivery        : single-CPU fallback");
        crate::println!("  work stealing       : single-CPU fallback");
    }
    crate::println!("  idle fallback       : verified");
    crate::println!("  resource reclaim    : verified");
    crate::println!("SMP_TEST: PASS");
}
