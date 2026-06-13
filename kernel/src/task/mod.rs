#[cfg(debug_assertions)]
mod m4c2_verify;
#[cfg(debug_assertions)]
mod m4c_verify;
mod stack;
mod wait_queue;

pub use wait_queue::{Completion, WaitQueue};

use alloc::{collections::VecDeque, vec::Vec};
#[cfg(debug_assertions)]
use core::{
    hint::{black_box, spin_loop},
    sync::atomic::AtomicBool,
};
use core::{
    marker::PhantomData,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
    smp::CpuId,
    tracked_spin::TrackedSpinLock,
};
use stack::KernelStack;

const MAX_TASKS: usize = 128;
const DEFAULT_TIME_SLICE_TICKS: u32 = 4;
const MAX_CPUS: usize = crate::smp::MAX_CPUS;
#[cfg(debug_assertions)]
const SINGLE_CPU_VERIFY_ITERATIONS: usize = 50_000;
#[cfg(debug_assertions)]
const SMP_VERIFY_ITERATIONS: usize = 25_000;
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
    _not_send: PhantomData<*mut ()>,
}

impl PreemptGuard {
    pub fn new() -> Self {
        preempt_disable();
        Self {
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
        preempt_enable();
    }
}

#[must_use = "dropping the guard re-enables migration"]
pub struct MigrationGuard {
    _preempt: PreemptGuard,
}

impl MigrationGuard {
    pub fn new() -> Self {
        Self {
            _preempt: PreemptGuard::new(),
        }
    }
}

impl Default for MigrationGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskId(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskState {
    Runnable,
    Running(CpuId),
    SwitchingOut(CpuId),
    Blocked,
    Idle(CpuId),
    Exited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskKind {
    Idle(CpuId),
    KernelThread,
    SystemThread,
}

impl TaskKind {
    const fn is_idle(self) -> bool {
        matches!(self, Self::Idle(_))
    }

    const fn is_counted_kernel_thread(self) -> bool {
        matches!(self, Self::KernelThread)
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
    wait_channel: Option<usize>,
    wake_after_switch: bool,
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
            wait_channel: None,
            wake_after_switch: false,
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
            wait_channel: None,
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
            state: TaskState::Runnable,
            context: crate::arch::task::Context::new(stack.top(), kernel_thread_bootstrap),
            stack: Some(stack),
            entry: Some(entry),
            affinity,
            queued_on: None,
            has_run: false,
            wait_channel: None,
            wake_after_switch: false,
        }
    }

    #[cfg(debug_assertions)]
    fn stack_contains(&self, address: usize) -> bool {
        self.stack
            .as_ref()
            .is_some_and(|stack| stack.contains(address))
    }

    fn destroy_resources(mut self) {
        if let Some(stack) = self.stack.take() {
            stack
                .destroy()
                .unwrap_or_else(|error| panic!("unable to release kernel stack: {error:?}"));
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
    disposition: SwitchDisposition,
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct WaiterDebugState {
    pub blocked: usize,
    pub switching: usize,
    pub claimed_switching: usize,
}

struct CpuScheduler {
    /// The CPU has installed its scheduler-owned idle context.
    online: bool,
    /// The CPU has entered the idle task with local interrupts enabled and may
    /// be selected as a normal scheduling or migration target.
    active: bool,
    current: Option<TaskId>,
    idle: Option<TaskId>,
    run_queue: VecDeque<TaskId>,
    pending: Option<PendingSwitch>,
    context_switches: u64,
    preemptions: u64,
    irq_depth: usize,
    preempt_count: usize,
    need_resched: bool,
    timeslice_remaining: u32,
}

impl CpuScheduler {
    fn new() -> Self {
        Self {
            online: false,
            active: false,
            current: None,
            idle: None,
            run_queue: VecDeque::with_capacity(MAX_TASKS),
            pending: None,
            context_switches: 0,
            preemptions: 0,
            irq_depth: 0,
            preempt_count: 0,
            need_resched: false,
            timeslice_remaining: DEFAULT_TIME_SLICE_TICKS,
        }
    }
}

struct Scheduler {
    tasks: Vec<Option<Task>>,
    retired_tasks: Vec<Task>,
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
        cpus[CpuId::BOOT.get()].active = true;
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

        let retired_tasks = Vec::with_capacity(MAX_TASKS);
        assert!(retired_tasks.capacity() >= MAX_TASKS);

        Self {
            tasks,
            retired_tasks,
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
            .filter(|cpu| self.cpus[cpu.get()].active)
            .min_by_key(|cpu| self.cpus[cpu.get()].run_queue.len())
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
                assert!(self.cpus[cpu.get()].online, "task target CPU is offline");
                assert!(
                    self.cpus[cpu.get()].active,
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

        self.enqueue(id, target);
        if request_reschedule {
            self.cpus[target.get()].need_resched = true;
        }
        if kind.is_counted_kernel_thread() {
            self.live_kernel_threads += 1;
        }
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

    fn steal_runnable(&mut self, cpu: CpuId) -> Option<TaskId> {
        for donor_index in 0..self.discovered_cpus {
            let donor = CpuId::new(donor_index).expect("invalid donor CPU");
            if donor == cpu || !self.cpus[donor.get()].active {
                continue;
            }

            let position = self.cpus[donor.get()].run_queue.iter().position(|id| {
                let task = self.task(*id);
                task.state == TaskState::Runnable && task.affinity.is_none()
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
        self.dequeue_local(cpu).or_else(|| self.steal_runnable(cpu))
    }

    fn activate_next(&mut self, id: TaskId, cpu: CpuId) {
        let task = self.task_mut(id);

        match task.kind {
            TaskKind::Idle(owner) => {
                assert_eq!(owner, cpu, "idle task selected by the wrong CPU");
                assert_eq!(task.state, TaskState::Idle(cpu));
            }
            TaskKind::KernelThread | TaskKind::SystemThread => {
                assert_eq!(task.state, TaskState::Runnable);
                if let Some(affinity) = task.affinity {
                    assert_eq!(affinity, cpu, "pinned task selected by the wrong CPU");
                }
                task.has_run = true;
            }
        }

        assert!(task.queued_on.is_none());
        task.state = TaskState::Running(cpu);

        self.cpus[cpu.get()].timeslice_remaining = DEFAULT_TIME_SLICE_TICKS;
        self.cpus[cpu.get()].need_resched = false;
    }

    fn prepare_yield(&mut self, cpu: CpuId) -> Option<ContextSwitch> {
        assert!(
            self.cpus[cpu.get()].active,
            "inactive CPU attempted to schedule"
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
        self.cpus[cpu.get()].need_resched = false;
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

    fn prepare_preempt(&mut self, cpu: CpuId) -> Option<ContextSwitch> {
        let cpu_state = &self.cpus[cpu.get()];
        assert!(cpu_state.active, "inactive CPU attempted to preempt");
        if cpu_state.irq_depth != 0 || cpu_state.preempt_count != 0 {
            return None;
        }
        assert!(
            cpu_state.pending.is_none(),
            "nested context switch attempted"
        );

        if !cpu_state.need_resched {
            return None;
        }

        let previous = self.current(cpu);
        assert_eq!(self.task(previous).state, TaskState::Running(cpu));

        let Some(next) = self.dequeue_next(cpu) else {
            self.cpus[cpu.get()].need_resched = false;
            self.cpus[cpu.get()].timeslice_remaining = DEFAULT_TIME_SLICE_TICKS;
            return None;
        };

        assert_ne!(previous, next, "CPU selected its current task as next");
        self.cpus[cpu.get()].need_resched = false;
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
        self.cpus[cpu.get()].preemptions = self.cpus[cpu.get()]
            .preemptions
            .checked_add(1)
            .expect("preemption counter overflowed");

        Some(self.context_pair(previous, next))
    }

    fn prepare_block(&mut self, cpu: CpuId, channel: usize) -> ContextSwitch {
        assert_ne!(channel, 0, "wait channel zero is reserved");
        assert!(
            self.cpus[cpu.get()].pending.is_none(),
            "nested switch attempted"
        );
        assert_eq!(
            self.cpus[cpu.get()].irq_depth,
            0,
            "IRQ context attempted to block"
        );
        assert_eq!(
            self.cpus[cpu.get()].preempt_count,
            0,
            "preemption-disabled task attempted to block",
        );

        let previous = self.current(cpu);
        let previous_task = self.task(previous);
        assert_eq!(previous_task.state, TaskState::Running(cpu));
        assert!(
            !previous_task.kind.is_idle(),
            "idle task attempted to block on a wait queue",
        );

        let next = self.dequeue_next(cpu).unwrap_or_else(|| self.idle(cpu));
        assert_ne!(previous, next);

        {
            let task = self.task_mut(previous);
            task.state = TaskState::SwitchingOut(cpu);
            task.wait_channel = Some(channel);
            task.wake_after_switch = false;
        }
        self.activate_next(next, cpu);
        self.cpus[cpu.get()].current = Some(next);
        self.cpus[cpu.get()].pending = Some(PendingSwitch {
            previous,
            disposition: SwitchDisposition::Block,
        });
        self.cpus[cpu.get()].context_switches = self.cpus[cpu.get()]
            .context_switches
            .checked_add(1)
            .expect("context switch counter overflowed");

        self.context_pair(previous, next)
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

    fn complete_switch(&mut self, cpu: CpuId) -> bool {
        let Some(pending) = self.cpus[cpu.get()].pending.take() else {
            return false;
        };

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
            }
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
                        task.state = TaskState::Runnable;
                    }
                    self.enqueue(pending.previous, cpu);
                    // The wake IPI may have arrived while local interrupts were
                    // disabled for the context switch. Preserve the scheduling
                    // request in software so progress does not depend on an
                    // interrupt-controller edge being replayed.
                    self.cpus[cpu.get()].need_resched = true;
                } else {
                    let task = self.task_mut(pending.previous);
                    assert!(
                        task.wait_channel.is_some(),
                        "blocking task reached schedule-tail without a wait channel",
                    );
                    task.state = TaskState::Blocked;
                }
            }
            SwitchDisposition::Exit => {
                self.task_mut(pending.previous).state = TaskState::Exited;
                if self.task(pending.previous).kind.is_counted_kernel_thread() {
                    self.live_kernel_threads = self
                        .live_kernel_threads
                        .checked_sub(1)
                        .expect("live kernel-thread counter underflowed");
                }

                let task = self.tasks[pending.previous.0]
                    .take()
                    .expect("exited task disappeared before reclamation");
                assert_eq!(task.id, pending.previous);
                assert_eq!(task.state, TaskState::Exited);
                assert!(
                    self.retired_tasks.len() < self.retired_tasks.capacity(),
                    "retired task queue exhausted",
                );
                self.retired_tasks.push(task);
                RETIRED_BACKLOG.fetch_add(1, Ordering::Release);
                return true;
            }
        }

        false
    }

    fn take_retired_task(&mut self) -> Option<Task> {
        let task = self.retired_tasks.pop()?;
        RETIRED_BACKLOG.fetch_sub(1, Ordering::AcqRel);
        Some(task)
    }

    #[cfg(debug_assertions)]
    fn clear_current_affinity(&mut self, cpu: CpuId) {
        let current = self.current(cpu);
        let task = self.task_mut(current);

        assert_eq!(task.state, TaskState::Running(cpu));
        assert!(
            !task.kind.is_idle(),
            "idle task affinity must remain fixed to its CPU",
        );
        task.affinity = None;
    }

    #[cfg(debug_assertions)]
    fn task_is_runnable_on(&self, id: TaskId, cpu: CpuId) -> bool {
        let task = self.task(id);
        task.state == TaskState::Runnable && task.queued_on == Some(cpu)
    }

    #[cfg(debug_assertions)]
    fn migrate_runnable_task(&mut self, id: TaskId, target: CpuId) -> CpuId {
        assert!(
            self.cpus[target.get()].active,
            "migration target CPU is not scheduler-active"
        );

        let (source, has_run, affinity) = {
            let task = self.task(id);
            assert_eq!(task.state, TaskState::Runnable);
            (
                task.queued_on
                    .expect("runnable migration task is not queued"),
                task.has_run,
                task.affinity,
            )
        };

        assert!(has_run, "migration test requires an already-run task");
        assert!(
            affinity.is_none() || affinity == Some(source),
            "task is pinned away from its migration source",
        );
        assert_ne!(source, target, "migration source and target are identical");

        let position = self.cpus[source.get()]
            .run_queue
            .iter()
            .position(|candidate| *candidate == id)
            .expect("migration task disappeared from its source run queue");
        let removed = self.cpus[source.get()]
            .run_queue
            .remove(position)
            .expect("migration task removal failed");
        assert_eq!(removed, id);

        {
            let task = self.task_mut(id);
            assert_eq!(task.queued_on, Some(source));
            task.queued_on = None;
            // Retarget affinity and queue ownership in the same scheduler
            // critical section. This prevents a work-stealing CPU from racing
            // the explicit hand-off between source removal and target enqueue.
            task.affinity = Some(target);
        }

        self.enqueue(id, target);
        self.cpus[target.get()].need_resched = true;
        source
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
        self.cpus[cpu.get()].active = false;
    }

    fn activate_secondary(&mut self, cpu: CpuId) {
        assert_ne!(cpu, CpuId::BOOT);
        assert!(cpu.get() < self.discovered_cpus);

        let idle = self.idle(cpu);
        assert_eq!(self.current(cpu), idle);
        assert_eq!(self.task(idle).state, TaskState::Running(cpu));

        let state = &mut self.cpus[cpu.get()];
        assert!(state.online, "offline CPU attempted to become active");
        assert!(!state.active, "secondary CPU became active twice");
        assert!(state.pending.is_none(), "CPU became active during a switch");
        state.active = true;
    }

    fn online_cpu_count(&self) -> usize {
        self.cpus
            .iter()
            .take(self.discovered_cpus)
            .filter(|cpu| cpu.online)
            .count()
    }

    fn online_cpu_mask(&self) -> usize {
        self.cpus
            .iter()
            .take(self.discovered_cpus)
            .enumerate()
            .fold(0_usize, |mask, (index, cpu)| {
                if cpu.online {
                    mask | (1_usize << index)
                } else {
                    mask
                }
            })
    }

    fn active_cpu_count(&self) -> usize {
        self.cpus
            .iter()
            .take(self.discovered_cpus)
            .filter(|cpu| cpu.active)
            .count()
    }

    fn active_cpu_mask(&self) -> usize {
        self.cpus
            .iter()
            .take(self.discovered_cpus)
            .enumerate()
            .fold(0_usize, |mask, (index, cpu)| {
                if cpu.active {
                    mask | (1_usize << index)
                } else {
                    mask
                }
            })
    }

    fn retired_task_count(&self) -> usize {
        self.retired_tasks.len()
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
                && self.cpus[donor.get()].active
                && self.cpus[donor.get()].run_queue.iter().any(|id| {
                    let task = self.task(*id);
                    task.state == TaskState::Runnable && task.affinity.is_none()
                })
        })
    }

    fn wake_waiters(&mut self, waiters: wait_queue::ClaimedWaiters) -> (usize, usize) {
        let mut target_mask = 0;
        let mut count = 0;

        for id in waiters.tasks.into_iter().take(waiters.count).flatten() {
            match self.task(id).state {
                TaskState::Blocked => {
                    let target = self
                        .task(id)
                        .affinity
                        .unwrap_or_else(|| self.choose_target_cpu());
                    {
                        let task = self.task_mut(id);
                        assert!(!task.wake_after_switch);
                        task.wait_channel = None;
                        task.state = TaskState::Runnable;
                    }
                    self.enqueue(id, target);
                    self.cpus[target.get()].need_resched = true;
                    target_mask |= 1_usize << target.get();
                    count += 1;
                }
                TaskState::SwitchingOut(cpu) => {
                    let task = self.task_mut(id);
                    assert!(!task.wake_after_switch, "waiter was claimed twice");
                    // This is the wait-queue equivalent of Linux's
                    // try_to_wake_up(): claim the sleep transition exactly
                    // once while the old context still owns its stack.
                    task.wake_after_switch = true;
                    target_mask |= 1_usize << cpu.get();
                    count += 1;
                }
                state => panic!("invalid waiter state during wakeup: {state:?}"),
            }
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

            match task.state {
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

    #[cfg(debug_assertions)]
    fn run_queue_len(&self, cpu: CpuId) -> usize {
        self.cpus[cpu.get()].run_queue.len()
    }

    fn irq_enter(&mut self, cpu: CpuId) {
        let state = &mut self.cpus[cpu.get()];
        assert!(state.online, "offline CPU entered IRQ context");
        state.irq_depth = state
            .irq_depth
            .checked_add(1)
            .expect("IRQ nesting counter overflowed");
    }

    fn irq_exit(&mut self, cpu: CpuId) -> bool {
        let state = &mut self.cpus[cpu.get()];
        state.irq_depth = state
            .irq_depth
            .checked_sub(1)
            .expect("IRQ nesting counter underflowed");
        state.irq_depth == 0 && state.preempt_count == 0 && state.need_resched
    }

    fn timer_tick(&mut self, cpu: CpuId) {
        let current = self.current(cpu);
        if self.task(current).kind.is_idle() {
            return;
        }

        let state = &mut self.cpus[cpu.get()];
        if state.timeslice_remaining > 1 {
            state.timeslice_remaining -= 1;
        } else {
            state.timeslice_remaining = 0;
            state.need_resched = true;
        }
    }

    fn request_reschedule(&mut self, cpu: CpuId) {
        if self.cpus[cpu.get()].active {
            self.cpus[cpu.get()].need_resched = true;
        }
    }

    fn preempt_disable(&mut self, cpu: CpuId) {
        let state = &mut self.cpus[cpu.get()];
        state.preempt_count = state
            .preempt_count
            .checked_add(1)
            .expect("preempt counter overflowed");
    }

    fn preempt_enable(&mut self, cpu: CpuId) -> bool {
        let state = &mut self.cpus[cpu.get()];
        state.preempt_count = state
            .preempt_count
            .checked_sub(1)
            .expect("preempt counter underflowed");
        state.preempt_count == 0 && state.irq_depth == 0 && state.need_resched
    }

    fn assert_schedulable(&self, cpu: CpuId) {
        let state = &self.cpus[cpu.get()];
        assert_eq!(
            state.irq_depth, 0,
            "task attempted to schedule in IRQ context"
        );
        assert_eq!(
            state.preempt_count, 0,
            "task attempted to schedule with preemption disabled",
        );
    }

    fn preempt_count(&self, cpu: CpuId) -> usize {
        self.cpus[cpu.get()].preempt_count
    }

    fn irq_depth(&self, cpu: CpuId) -> usize {
        self.cpus[cpu.get()].irq_depth
    }

    fn can_preempt_in_task_context(&self, cpu: CpuId) -> bool {
        self.cpus[cpu.get()].irq_depth == 0
    }

    fn context_switches_total(&self) -> u64 {
        self.cpus
            .iter()
            .take(self.discovered_cpus)
            .map(|cpu| cpu.context_switches)
            .sum()
    }

    fn preemptions_total(&self) -> u64 {
        self.cpus
            .iter()
            .take(self.discovered_cpus)
            .map(|cpu| cpu.preemptions)
            .sum()
    }
}

static SCHEDULER: IrqSpinLock<Option<Scheduler>> =
    IrqSpinLock::new_with_class(None, LockClass::new("scheduler", LockRank::Scheduler, 1));
static RETIRED_REAPER: TrackedSpinLock<()> =
    TrackedSpinLock::new_with_class((), LockClass::new("retired_reaper", LockRank::CrossCpu, 1));
static TASK_REAPER_QUEUE: WaitQueue = WaitQueue::new();
static RETIRED_BACKLOG: AtomicUsize = AtomicUsize::new(0);
static IDLE_ENTERS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static IDLE_EXITS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

pub fn initialize() {
    let discovered = crate::smp::discovered_cpu_count();
    let scheduler = Scheduler::new(discovered);

    {
        let mut slot = SCHEDULER.lock();

        assert!(slot.is_none(), "kernel scheduler was initialized twice");
        *slot = Some(scheduler);
    }

    spawn_system_thread(task_reaper_main, Some(CpuId::BOOT), Some(CpuId::BOOT));

    crate::println!("kernel scheduler:");
    crate::println!("  policy          : preemptive per-CPU FIFO round-robin");
    crate::println!("  kernel stack    : 16 KiB plus guard pages");
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
    let mut slot = SCHEDULER.lock();
    if let Some(scheduler) = slot.as_mut() {
        scheduler.irq_enter(cpu);
    }
}

pub fn irq_exit() {
    let cpu = crate::smp::current_cpu_id();
    let should_preempt = {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .is_some_and(|scheduler| scheduler.irq_exit(cpu))
    };

    if should_preempt {
        preempt_schedule_irq();
    }
}

pub fn on_timer_tick() {
    let cpu = crate::smp::current_cpu_id();
    let mut slot = SCHEDULER.lock();
    if let Some(scheduler) = slot.as_mut() {
        scheduler.timer_tick(cpu);
    }
}

pub fn request_reschedule_local() {
    let cpu = crate::smp::current_cpu_id();
    let mut slot = SCHEDULER.lock();
    if let Some(scheduler) = slot.as_mut() {
        scheduler.request_reschedule(cpu);
    }
}

fn request_reschedule_on(cpu: CpuId) {
    {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .request_reschedule(cpu);
    }

    if cpu != crate::smp::current_cpu_id() {
        crate::smp::send_ipi(cpu);
    }
}

pub fn preempt_disable() {
    let cpu = crate::smp::current_cpu_id();
    let mut slot = SCHEDULER.lock();
    slot.as_mut()
        .expect("preemption used before scheduler initialization")
        .preempt_disable(cpu);
}

pub fn preempt_enable() {
    let cpu = crate::smp::current_cpu_id();
    let should_schedule_now = {
        let mut slot = SCHEDULER.lock();
        let scheduler = slot
            .as_mut()
            .expect("preemption used before scheduler initialization");
        scheduler.preempt_enable(cpu) && scheduler.can_preempt_in_task_context(cpu)
    };

    if should_schedule_now && crate::arch::interrupt::are_enabled() {
        preempt_schedule();
    }
}

pub fn preempt_count() -> usize {
    let cpu = crate::smp::current_cpu_id();
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("preemption queried before scheduler initialization")
        .preempt_count(cpu)
}

pub fn irq_depth() -> usize {
    let cpu = crate::smp::current_cpu_id();
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("IRQ depth queried before scheduler initialization")
        .irq_depth(cpu)
}

pub(super) fn block_current_on_if<F>(queue: &WaitQueue, should_block: F) -> bool
where
    F: FnOnce() -> bool,
{
    crate::context::might_sleep();

    let interrupt_guard = crate::context::IrqSaveGuard::new();
    let cpu = crate::smp::current_cpu_id();
    let switch = {
        let mut slot = SCHEDULER.lock();
        let scheduler = slot.as_mut().expect("kernel scheduler is not initialized");
        scheduler.assert_schedulable(cpu);

        if should_block() {
            let current = scheduler.current(cpu);
            let channel = queue.channel();
            queue.enqueue_current(current);
            Some(scheduler.prepare_block(cpu, channel))
        } else {
            None
        }
    };

    let Some((previous, next)) = switch else {
        return false;
    };

    #[cfg(debug_assertions)]
    m4c_verify::before_block_context_switch();

    // SAFETY: the outgoing task is held in SwitchingOut until the incoming
    // context completes the switch. The wait channel remains attached to it,
    // so a concurrent wakeup becomes wake_after_switch instead of being lost.
    unsafe { crate::arch::task::switch(previous, next) };

    finish_switch();
    drop(interrupt_guard);
    reap_retired_tasks();
    true
}

pub(super) fn wake_queue(queue: &WaitQueue, maximum: usize) -> usize {
    let (woken, targets) = {
        let mut slot = SCHEDULER.lock();
        let scheduler = slot.as_mut().expect("kernel scheduler is not initialized");
        let waiters = queue.claim_waiters(maximum);
        scheduler.wake_waiters(waiters)
    };

    let current = crate::smp::current_cpu_id();
    for index in 0..crate::smp::discovered_cpu_count() {
        let bit = 1_usize << index;
        if targets & bit == 0 {
            continue;
        }

        let cpu = CpuId::new(index).expect("wakeup target exceeds MAX_CPUS");
        if cpu != current {
            crate::smp::send_ipi(cpu);
        }
    }

    woken
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
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .run_queue_len(cpu)
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

    let mut slot = SCHEDULER.lock();
    slot.as_mut()
        .expect("kernel scheduler is not initialized")
        .register_secondary(cpu);
}

fn mark_current_active() {
    assert!(
        crate::arch::interrupt::are_enabled(),
        "CPU became scheduler-active with local interrupts disabled",
    );

    let cpu = crate::smp::current_cpu_id();
    let mut slot = SCHEDULER.lock();
    slot.as_mut()
        .expect("kernel scheduler is not initialized")
        .activate_secondary(cpu);
}

pub fn finalize_cpu_bringup() {
    assert_eq!(crate::smp::current_cpu_id(), CpuId::BOOT);
    assert!(crate::arch::interrupt::are_enabled());

    let (registered, registered_mask, active, active_mask) = {
        let slot = SCHEDULER.lock();
        let scheduler = slot.as_ref().expect("kernel scheduler is not initialized");
        (
            scheduler.online_cpu_count(),
            scheduler.online_cpu_mask(),
            scheduler.active_cpu_count(),
            scheduler.active_cpu_mask(),
        )
    };

    let smp_online = crate::smp::online_cpu_count();
    let smp_online_mask = crate::smp::online_cpu_mask();
    let smp_ready_mask = crate::smp::ipi_ready_cpu_mask();
    assert_eq!(registered, smp_online, "scheduler/SMP online CPU mismatch");
    assert_eq!(
        registered_mask, smp_online_mask,
        "scheduler/SMP online CPU masks diverged",
    );
    assert_eq!(
        active, smp_online,
        "not every online CPU became scheduler-active"
    );
    assert_eq!(
        active_mask, smp_ready_mask,
        "scheduler-active and IPI-ready CPU masks diverged",
    );

    crate::println!("kernel scheduler CPUs:");
    crate::println!("  registered CPUs : {}", registered);
    crate::println!("  active CPUs     : {}", active);
    crate::println!("  active mask     : {:#x}", active_mask);
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
    let switch = {
        let mut slot = SCHEDULER.lock();
        let scheduler = slot.as_mut().expect("kernel scheduler is not initialized");
        scheduler.assert_schedulable(cpu);
        scheduler.prepare_yield(cpu)
    };

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
    let cpu = crate::smp::current_cpu_id();
    let switch = {
        let mut slot = SCHEDULER.lock();
        let Some(scheduler) = slot.as_mut() else {
            return;
        };
        scheduler.prepare_preempt(cpu)
    };

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
    let cpu = crate::smp::current_cpu_id();
    let (previous, next) = {
        let mut slot = SCHEDULER.lock();
        let scheduler = slot.as_mut().expect("kernel scheduler is not initialized");
        scheduler.assert_schedulable(cpu);
        scheduler.prepare_exit(cpu)
    };

    // SAFETY: the exiting task remains allocated and marked SwitchingOut
    // until the incoming context calls finish_switch() from a different stack.
    unsafe { crate::arch::task::switch(previous, next) };

    panic!("exited kernel thread resumed unexpectedly");
}

fn finish_switch() {
    let cpu = crate::smp::current_cpu_id();
    let retired_task_added = {
        let mut slot = SCHEDULER.lock();
        slot.as_mut()
            .expect("kernel scheduler is not initialized")
            .complete_switch(cpu)
    };

    if retired_task_added {
        TASK_REAPER_QUEUE.wake_one();
    }
}

fn drain_retired_queue() {
    loop {
        let retired = {
            let mut slot = SCHEDULER.lock();
            slot.as_mut()
                .expect("kernel scheduler is not initialized")
                .take_retired_task()
        };

        let Some(task) = retired else {
            break;
        };

        task.destroy_resources();
    }
}

fn task_reaper_main() {
    loop {
        TASK_REAPER_QUEUE.wait_until(|| retired_task_backlog() != 0);

        let reaper = RETIRED_REAPER.lock();
        drain_retired_queue();
        drop(reaper);
    }
}

fn reap_retired_tasks() {
    crate::context::might_sleep();
    if retired_task_backlog() != 0 {
        TASK_REAPER_QUEUE.wake_one();
    }
}

#[cfg(debug_assertions)]
fn synchronize_retired_tasks() {
    crate::context::might_sleep();
    assert_eq!(
        live_kernel_threads(),
        0,
        "task reclamation barrier requires a quiescent verifier",
    );

    TASK_REAPER_QUEUE.wake_one();
    while retired_task_backlog() != 0 {
        yield_now();
    }

    let reaper = RETIRED_REAPER.lock();
    drop(reaper);

    let retired = retired_task_count();
    assert_eq!(retired, 0, "retired task queue was not fully drained");
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
fn active_cpu_count() -> usize {
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .active_cpu_count()
}

#[cfg(debug_assertions)]
fn active_cpu_mask() -> usize {
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .active_cpu_mask()
}

// This counter is part of the runtime reaper/idle protocol, not a verifier-only
// diagnostic. Keep it available in release builds as well.
fn retired_task_backlog() -> usize {
    RETIRED_BACKLOG.load(Ordering::Acquire)
}

#[cfg(debug_assertions)]
fn retired_task_count() -> usize {
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .retired_task_count()
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

#[cfg(debug_assertions)]
pub(super) fn preemptions() -> u64 {
    let slot = SCHEDULER.lock();
    slot.as_ref()
        .expect("kernel scheduler is not initialized")
        .preemptions_total()
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

    // SAFETY: secondary initialization installed its trap vector, local timer,
    // IPI source and permanent guarded idle stack before entering this context.
    unsafe { crate::arch::interrupt::enable() };

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

    if current_cpu_has_work() || retired_task_backlog() != 0 {
        // SAFETY: this CPU is already in a fully initialized idle context; the
        // caller will immediately leave the idle path and schedule/reap work.
        unsafe { crate::arch::interrupt::enable() };
        IDLE_EXITS[crate::smp::current_cpu_id().get()].fetch_add(1, Ordering::AcqRel);
        return;
    }

    let cpu = crate::smp::current_cpu_id();
    IDLE_ENTERS[cpu.get()].fetch_add(1, Ordering::AcqRel);

    // SAFETY: the idle task has a valid trap frame, local interrupt sources are
    // configured, and work was rechecked with local interrupts disabled.
    unsafe { crate::arch::cpu::enable_and_wait_for_interrupt() };

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
        // Remote completion counters are plain atomic publications, not wake
        // events. Keep this debug verifier runnable instead of relying on a
        // periodic timer to escape WFI.
        yield_now();
        spin_loop();
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
    let worker_count = if cpu_count == 1 { 2 } else { cpu_count };
    let iterations = if cpu_count == 1 {
        SINGLE_CPU_VERIFY_ITERATIONS
    } else {
        SMP_VERIFY_ITERATIONS
    };

    VERIFY_ITERATIONS.store(iterations, Ordering::Release);
    USE_CONCURRENT_BARRIER.store(cpu_count > 1, Ordering::Release);
    EXPECTED_WORKER_MASK.store((1_usize << worker_count) - 1, Ordering::Release);

    let pages_before = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable before scheduler verification");
    let switches_before = context_switches();

    let preempt_guard = PreemptGuard::new();
    {
        for index in 0..worker_count {
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

        for index in 0..worker_count {
            let cpu = if cpu_count == 1 {
                CpuId::BOOT
            } else {
                CpuId::new(index).expect("worker CPU exceeds MAX_CPUS")
            };
            request_reschedule_on(cpu);
        }
    }
    drop(preempt_guard);

    wait_for_workers(worker_count);
    verify_ipi_delivery(cpu_count);
    verify_work_stealing(cpu_count);
    synchronize_retired_tasks();
    crate::heap::shrink();

    let switches = context_switches()
        .checked_sub(switches_before)
        .expect("context switch counter moved backwards");
    let minimum_switches = (iterations as u64)
        .checked_mul(worker_count as u64)
        .expect("scheduler switch threshold overflowed");
    let pages_after = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable after scheduler verification");
    let (idle_enters, idle_exits) = idle_counter_totals();

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

    assert!(
        switches >= minimum_switches,
        "too few context switches: actual={switches} minimum={minimum_switches}",
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
    m4c2_verify::verify();

    crate::println!("kernel scheduler test:");
    crate::println!("  kernel threads  : verified ({})", worker_count);
    crate::println!("  private stacks  : verified");
    crate::println!("  context switch  : verified ({} switches)", switches);
    crate::println!("  cooperative     : verified");
    crate::println!("  timer coexistence: verified");
    crate::println!("  task exit       : verified");
    crate::println!("  resource reclaim: verified");
    crate::println!(
        "  idle protocol   : enters={} exits={}",
        idle_enters,
        idle_exits,
    );

    crate::println!("SMP scheduler test:");
    crate::println!("  participating CPUs : {}", cpu_count);
    crate::println!("  concurrent threads : verified");
    crate::println!("  per-CPU current     : verified");
    crate::println!("  task affinity       : verified");
    if cpu_count > 1 {
        crate::println!("  remote wakeup       : verified");
        crate::println!("  IPI delivery        : verified");
        crate::println!("  work stealing       : verified (runnable task migration)");
    } else {
        crate::println!("  remote wakeup       : single-CPU fallback");
        crate::println!("  IPI delivery        : single-CPU fallback");
        crate::println!("  work stealing       : single-CPU fallback");
    }
    crate::println!("  idle fallback       : verified");
    crate::println!("  resource reclaim    : verified");
    crate::println!("SMP_TEST: PASS");
}
