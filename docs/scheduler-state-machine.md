# Scheduler State Machine

This document freezes the current M4C/M4C2 scheduler baseline.

## Task States

```text
Runnable
  -> Running(cpu)
  -> SwitchingOut(cpu)
  -> Runnable | Blocked | Exited | Idle(cpu)

Blocked
  -> Runnable

Idle(cpu)
  -> Running(cpu)
  -> SwitchingOut(cpu)
  -> Idle(cpu)
```

`SwitchingOut` is a real state. It protects the outgoing stack while the CPU has
selected the next context but has not yet completed schedule-tail.

## Blocking and Wakeup

Current wait queues own an explicit waiter list. Blocking still records a
numeric channel in the task for schedule-tail diagnostics, but wakeup no longer
scans the whole task table. A waker claims entries from the queue list, then
transitions those exact tasks under the scheduler lock.

The important invariant:

```text
Running task decides to block
  -> enqueue current task on the WaitQueue list
  -> state = SwitchingOut, wait_channel = channel
  -> scheduler lock drops and context switch begins
  -> a waker may claim the SwitchingOut waiter
  -> schedule-tail converts it back to Runnable instead of losing the wakeup
```

## What M4C Verifies

`kernel/src/task/m4c_verify.rs` proves:

- timer preemption can preempt a non-yielding kernel thread;
- `preempt_disable()` prevents timer preemption until re-enabled;
- blocking wait queues wake one and wake all correctly;
- wake-before-sleep rechecks the condition under the scheduler critical section;
- wake-during-switch does not lose a waiter in `SwitchingOut`;
- `Completion::complete()` wakes one waiter;
- `Completion::complete_all()` releases all current and future waiters;
- task resources are reclaimed after the verifier becomes quiescent.

It does not prove:

- timeout wait behavior;
- signal or process-kill semantics;
- external device IRQ wakeups.

## What M4C2 Verifies

`kernel/src/task/m4c2_verify.rs` proves:

- remote CPUs can prime and later observe a remapped kernel virtual address;
- kernel-wide TLB shootdown sends IPIs and waits for remote ACKs;
- stale remote TLB entries are not retained after backing page replacement;
- an already-started task can migrate from CPU0 to CPU1;
- migration preserves stack and resource lifetime safety.

It does not prove:

- per-process address-space shootdown;
- ASID or range flush correctness;
- CPU hotplug interactions with TLB target masks;
- user-mode page fault recovery.

## Current Scheduler Policy

- Per-CPU FIFO run queues.
- Global scheduler IRQ spinlock.
- Idle tasks can steal unpinned runnable work from active CPUs.
- Timer slices request preemption after `DEFAULT_TIME_SLICE_TICKS`.
- Exited tasks are retired and later drained by the reaper path.
- Task stacks are destroyed explicitly by the dedicated task reaper thread.
  `KernelStack::Drop` only verifies that explicit teardown already happened.
- Idle uses a disable/recheck/enable-and-wait protocol and reports aggregate
  enter/exit counters in the scheduler verifier log.
