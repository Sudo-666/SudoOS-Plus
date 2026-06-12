mod stack;

use alloc::{collections::VecDeque, vec::Vec};
#[cfg(debug_assertions)]
use core::{
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::irq_lock::IrqSpinLock;
use stack::KernelStack;

const MAX_TASKS: usize = 64;
#[cfg(debug_assertions)]
const VERIFY_ITERATIONS: usize = 40_000;

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
    Running,
    Exited,
}

struct Task {
    id: TaskId,
    state: TaskState,
    context: crate::arch::task::Context,
    stack: Option<KernelStack>,
    entry: Option<KernelThreadEntry>,
}

impl Task {
    fn boot() -> Self {
        Self {
            id: TaskId(0),
            state: TaskState::Running,
            context: crate::arch::task::Context::default(),
            stack: None,
            entry: None,
        }
    }

    fn kernel_thread(id: TaskId, entry: KernelThreadEntry, stack: KernelStack) -> Self {
        let context = crate::arch::task::Context::new(stack.top(), kernel_thread_bootstrap);

        Self {
            id,
            state: TaskState::Runnable,
            context,
            stack: Some(stack),
            entry: Some(entry),
        }
    }

    #[cfg(debug_assertions)]
    fn stack_contains(&self, address: usize) -> bool {
        self.stack
            .as_ref()
            .is_some_and(|stack| stack.contains(address))
    }
}

struct Scheduler {
    tasks: Vec<Option<Task>>,
    run_queue: VecDeque<TaskId>,
    zombies: VecDeque<TaskId>,
    current: TaskId,
    context_switches: u64,
    live_kernel_threads: usize,
}

impl Scheduler {
    fn new() -> Self {
        let mut tasks = Vec::with_capacity(MAX_TASKS);
        tasks.push(Some(Task::boot()));
        assert!(tasks.capacity() >= MAX_TASKS);

        Self {
            tasks,
            run_queue: VecDeque::with_capacity(MAX_TASKS),
            zombies: VecDeque::with_capacity(MAX_TASKS),
            current: TaskId(0),
            context_switches: 0,
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

    fn spawn(&mut self, entry: KernelThreadEntry, stack: KernelStack) -> TaskId {
        let reusable = self
            .tasks
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(index, slot)| slot.is_none().then_some(index));

        let id = match reusable {
            Some(index) => TaskId(index),
            None => {
                assert!(
                    self.tasks.len() < MAX_TASKS,
                    "kernel task table exhausted: capacity={MAX_TASKS}",
                );
                TaskId(self.tasks.len())
            }
        };

        let task = Task::kernel_thread(id, entry, stack);

        if id.0 == self.tasks.len() {
            self.tasks.push(Some(task));
        } else {
            assert!(self.tasks[id.0].is_none());
            self.tasks[id.0] = Some(task);
        }

        self.run_queue.push_back(id);
        self.live_kernel_threads += 1;

        id
    }

    fn prepare_yield(&mut self) -> Option<ContextSwitch> {
        let next = self.run_queue.pop_front()?;
        let previous = self.current;

        assert_ne!(previous, next, "running task appeared in its own run queue");
        assert_eq!(self.task(previous).state, TaskState::Running);
        assert_eq!(self.task(next).state, TaskState::Runnable);

        self.task_mut(previous).state = TaskState::Runnable;
        self.run_queue.push_back(previous);
        self.task_mut(next).state = TaskState::Running;
        self.current = next;
        self.context_switches = self
            .context_switches
            .checked_add(1)
            .expect("context switch counter overflowed");

        Some(self.context_pair(previous, next))
    }

    fn prepare_exit(&mut self) -> ContextSwitch {
        let previous = self.current;
        let next = self
            .run_queue
            .pop_front()
            .expect("the last runnable task attempted to exit");

        assert_ne!(previous, TaskId(0), "boot task must never exit");
        assert_eq!(self.task(previous).state, TaskState::Running);
        assert_eq!(self.task(next).state, TaskState::Runnable);

        self.task_mut(previous).state = TaskState::Exited;
        self.zombies.push_back(previous);
        self.live_kernel_threads = self
            .live_kernel_threads
            .checked_sub(1)
            .expect("live kernel-thread counter underflowed");

        self.task_mut(next).state = TaskState::Running;
        self.current = next;
        self.context_switches = self
            .context_switches
            .checked_add(1)
            .expect("context switch counter overflowed");

        self.context_pair(previous, next)
    }

    fn context_pair(&mut self, previous: TaskId, next: TaskId) -> ContextSwitch {
        assert_ne!(previous, next);

        let previous_pointer = {
            let previous_task = self.task_mut(previous);
            core::ptr::addr_of_mut!(previous_task.context)
        };

        let next_pointer = {
            let next_task = self.task(next);
            core::ptr::addr_of!(next_task.context)
        };

        (previous_pointer, next_pointer)
    }

    fn take_one_zombie(&mut self) -> Option<Task> {
        let id = self.zombies.pop_front()?;
        let task = self
            .tasks
            .get_mut(id.0)
            .and_then(Option::take)
            .expect("zombie task disappeared before reclamation");

        assert_eq!(task.id, id);
        assert_eq!(task.state, TaskState::Exited);

        Some(task)
    }
}

static SCHEDULER: IrqSpinLock<Option<Scheduler>> = IrqSpinLock::new(None);

pub fn initialize() {
    let mut slot = SCHEDULER.lock();

    assert!(slot.is_none(), "kernel scheduler was initialized twice");
    *slot = Some(Scheduler::new());

    crate::println!("kernel scheduler:");
    crate::println!("  policy          : FIFO round-robin");
    crate::println!("  kernel stack    : 16 KiB plus guard pages");
    crate::println!("  active CPUs     : 1");
    crate::println!("  SMP foundation  : task/context ABI ready; secondary CPUs pending");
}

pub fn spawn_kernel_thread(entry: KernelThreadEntry) -> TaskId {
    let stack = KernelStack::allocate()
        .unwrap_or_else(|error| panic!("unable to allocate kernel-thread stack: {error:?}"));

    let mut slot = SCHEDULER.lock();
    let scheduler = slot.as_mut().expect("kernel scheduler is not initialized");

    scheduler.spawn(entry, stack)
}

pub fn yield_now() {
    assert!(
        crate::arch::interrupt::are_enabled(),
        "yield_now requires local interrupts to be enabled",
    );

    let interrupt_state = crate::arch::interrupt::save_and_disable();

    let switch = {
        let mut slot = SCHEDULER.lock();
        let scheduler = slot.as_mut().expect("kernel scheduler is not initialized");
        scheduler.prepare_yield()
    };

    let Some((previous, next)) = switch else {
        crate::arch::interrupt::restore(interrupt_state);
        return;
    };

    // SAFETY: scheduler state marks `next` running and keeps both task
    // contexts alive. Local interrupts remain disabled until the switch has
    // completed and this task later resumes.
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

    let (previous, next) = {
        let mut slot = SCHEDULER.lock();
        let scheduler = slot.as_mut().expect("kernel scheduler is not initialized");
        scheduler.prepare_exit()
    };

    // SAFETY: the exiting task stays allocated as a zombie until execution is
    // already on the next task's stack. The scheduler selected a distinct,
    // live runnable context and local interrupts are disabled.
    unsafe { crate::arch::task::switch(previous, next) };

    panic!("exited kernel thread resumed unexpectedly");
}

fn finish_switch() {
    loop {
        let zombie = {
            let mut slot = SCHEDULER.lock();
            let scheduler = slot.as_mut().expect("kernel scheduler is not initialized");
            scheduler.take_one_zombie()
        };

        let Some(task) = zombie else {
            return;
        };

        /*
         * Drop outside the scheduler lock. Releasing a vmalloc-backed stack
         * acquires the VM and page-allocator locks.
         */
        drop(task);
    }
}

fn current_entry() -> KernelThreadEntry {
    let slot = SCHEDULER.lock();
    let scheduler = slot.as_ref().expect("kernel scheduler is not initialized");

    scheduler
        .task(scheduler.current)
        .entry
        .expect("current task is not a kernel thread")
}

#[cfg(debug_assertions)]
fn current_stack_contains(address: usize) -> bool {
    let slot = SCHEDULER.lock();
    let scheduler = slot.as_ref().expect("kernel scheduler is not initialized");

    scheduler.task(scheduler.current).stack_contains(address)
}

#[cfg(debug_assertions)]
fn live_kernel_threads() -> usize {
    let slot = SCHEDULER.lock();
    let scheduler = slot.as_ref().expect("kernel scheduler is not initialized");

    scheduler.live_kernel_threads
}

#[cfg(debug_assertions)]
fn context_switches() -> u64 {
    let slot = SCHEDULER.lock();
    let scheduler = slot.as_ref().expect("kernel scheduler is not initialized");

    scheduler.context_switches
}

unsafe extern "C" fn kernel_thread_bootstrap() -> ! {
    finish_switch();

    // SAFETY: the scheduler always enters a fresh kernel thread with a valid
    // trap vector, timer source, and guarded kernel stack installed.
    unsafe { crate::arch::interrupt::enable() };

    let entry = current_entry();
    entry();
    exit_current()
}

#[cfg(debug_assertions)]
static THREAD_A_PROGRESS: AtomicUsize = AtomicUsize::new(0);
#[cfg(debug_assertions)]
static THREAD_B_PROGRESS: AtomicUsize = AtomicUsize::new(0);
#[cfg(debug_assertions)]
static THREAD_A_STACK: AtomicUsize = AtomicUsize::new(0);
#[cfg(debug_assertions)]
static THREAD_B_STACK: AtomicUsize = AtomicUsize::new(0);
#[cfg(debug_assertions)]
static COMPLETED_THREADS: AtomicUsize = AtomicUsize::new(0);

#[cfg(debug_assertions)]
fn verification_thread_a() {
    let canary = 0x1357_2468_aaaa_5555_usize;
    let address = core::ptr::addr_of!(canary) as usize;

    assert!(current_stack_contains(address));
    THREAD_A_STACK.store(address, Ordering::Release);

    for iteration in 0..VERIFY_ITERATIONS {
        assert_eq!(black_box(canary), 0x1357_2468_aaaa_5555_usize);
        THREAD_A_PROGRESS.store(iteration + 1, Ordering::Release);
        yield_now();
    }

    COMPLETED_THREADS.fetch_add(1, Ordering::AcqRel);
}

#[cfg(debug_assertions)]
fn verification_thread_b() {
    let canary = 0xfedc_ba98_5555_aaaa_usize;
    let address = core::ptr::addr_of!(canary) as usize;

    assert!(current_stack_contains(address));
    THREAD_B_STACK.store(address, Ordering::Release);

    for iteration in 0..VERIFY_ITERATIONS {
        assert_eq!(black_box(canary), 0xfedc_ba98_5555_aaaa_usize);
        THREAD_B_PROGRESS.store(iteration + 1, Ordering::Release);
        yield_now();
    }

    COMPLETED_THREADS.fetch_add(1, Ordering::AcqRel);
}

#[cfg(debug_assertions)]
pub fn verify() {
    THREAD_A_PROGRESS.store(0, Ordering::Release);
    THREAD_B_PROGRESS.store(0, Ordering::Release);
    THREAD_A_STACK.store(0, Ordering::Release);
    THREAD_B_STACK.store(0, Ordering::Release);
    COMPLETED_THREADS.store(0, Ordering::Release);

    crate::heap::shrink();

    let pages_before = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable before scheduler verification");
    let ticks_before = crate::time::timer_ticks();
    let switches_before = context_switches();

    let first = spawn_kernel_thread(verification_thread_a);
    let second = spawn_kernel_thread(verification_thread_b);
    assert_ne!(first, second);

    while COMPLETED_THREADS.load(Ordering::Acquire) != 2 || live_kernel_threads() != 0 {
        yield_now();
    }

    finish_switch();
    crate::heap::shrink();

    let target_tick = ticks_before
        .checked_add(2)
        .expect("timer verification target overflowed");
    while crate::time::timer_ticks() < target_tick {
        crate::arch::cpu::wait_for_interrupt();
    }

    let switches = context_switches()
        .checked_sub(switches_before)
        .expect("context switch counter moved backwards");
    let first_stack = THREAD_A_STACK.load(Ordering::Acquire);
    let second_stack = THREAD_B_STACK.load(Ordering::Acquire);
    let pages_after = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable after scheduler verification");

    assert_eq!(THREAD_A_PROGRESS.load(Ordering::Acquire), VERIFY_ITERATIONS);
    assert_eq!(THREAD_B_PROGRESS.load(Ordering::Acquire), VERIFY_ITERATIONS);
    assert_ne!(first_stack, 0);
    assert_ne!(second_stack, 0);
    assert_ne!(first_stack, second_stack);
    assert!(switches >= 100_000, "too few context switches: {switches}");
    assert_eq!(pages_before, pages_after, "kernel task resources leaked");
    assert!(crate::arch::interrupt::are_enabled());

    crate::println!("kernel scheduler test:");
    crate::println!("  kernel threads  : verified (2)");
    crate::println!("  private stacks  : verified");
    crate::println!("  context switch  : verified ({} switches)", switches);
    crate::println!("  cooperative     : verified");
    crate::println!("  timer coexistence: verified");
    crate::println!("  task exit       : verified");
    crate::println!("  resource reclaim: verified");
}
