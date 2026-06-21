# Context Rules

SudoOS currently has these execution contexts:

- early boot: CPU 0 only, before all normal kernel services exist.
- task context: ordinary kernel thread execution with local interrupts enabled.
- idle context: per-CPU idle task, allowed to reap retired resources.
- hardirq context: timer or software interrupt handler.
- panic context: one CPU owns the report; other CPUs must stop safely.

## Current Rules

| Operation | early boot | task | idle | hardirq | panic |
|-----------|------------|------|------|---------|-------|
| allocate heap | after heap init | yes | yes | no | no |
| block/sleep | no | yes | no | no | no |
| yield/schedule | no | yes | yes | IRQ-return only | no |
| synchronous TLB shootdown | local only before SMP | yes | yes | no | no |
| reclaim task stacks | no | reaper thread only | reaper wake only | no | no |
| send reschedule IPI | after IPI-ready | yes | yes | limited | no |

## Existing Enforcement

- `IrqSpinLock` saves and disables local interrupts while holding protected
  scheduler, SMP and VM metadata.
- Scheduler APIs assert against blocking in IRQ context and against scheduling
  with preemption disabled.
- Multi-CPU TLB shootdown asserts local interrupts are enabled.
- Wait queue blocking requires local interrupts enabled.

## Implemented Guard API

The first PR 2 slice provides:

- `context::IrqSaveGuard`
- `context::assert_interrupts_enabled()`
- `context::assert_interrupts_disabled()`
- `context::in_irq()`
- `context::irq_depth()`
- `context::preempt_count()`
- `context::assert_task_context()`
- `context::assert_irq_context()`
- `context::might_sleep()`
- `task::PreemptGuard`
- `task::MigrationGuard`

`MigrationGuard` is currently implemented as a preemption guard. It intentionally
has a separate type so future active-mm and task migration rules can grow behind
the same API.

Current users:

- `IrqSpinLock` restores interrupts through `IrqSaveGuard`.
- wait queue blocking uses `might_sleep()`.
- retired task synchronization uses `might_sleep()` and `PreemptGuard`.
- synchronous TLB shootdown uses `MigrationGuard` while sampling current CPU and
  waiting for ACKs.
- page freeing uses an allocator `Freeing` quarantine state so debug poison runs
  outside the allocator lock without exposing the page to new allocations.
- exited task stacks use explicit `KernelStack::destroy()` in the dedicated
  task reaper. Dropping a live stack without explicit destroy is a bug.
- idle uses disable/recheck/enable-and-wait so work arriving between the idle
  check and the architectural sleep is delivered by timer/IPI wakeup.

## Remaining M5 Follow-Up

Later M5 slices should extend these rules with:

- compile-time or debug-only checks that discourage new naked
  `preempt_disable()` / `preempt_enable()` pairs;
- scheduler recursion diagnostics.

New code should prefer RAII guards over naked disable/enable pairs.
