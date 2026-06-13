# CPU Lifecycle

CPU identity and CPU runtime state are deliberately separate.

The logical-to-hardware identity map is constructed once from firmware and
published as an immutable lockless snapshot. Runtime readiness is represented
by one authoritative atomic state per logical CPU.

## States

```text
Absent
  -> Present
  -> Starting
  -> SchedulerRegistered
  -> Active
  -> IpiReady

Starting -> Failed

IpiReady -> Dying -> Dead        (reserved for future hotplug)
```

Every transition uses an exact-state atomic compare/exchange. Skipping a state,
publishing twice, or publishing from the wrong context panics.

## Derived Masks

There are no independently writable online, active or IPI-ready masks.

`online_cpu_mask()`, `scheduler_active_cpu_mask()` and
`ipi_ready_cpu_mask()` scan the authoritative state array and derive snapshots.

## Boot CPU

```text
smp::initialize                 Present
task::initialize                SchedulerRegistered -> Active
smp::start_secondaries          IpiReady
```

## Secondary CPU

```text
boot start request              Present -> Starting
scheduler context installed     Starting -> SchedulerRegistered
idle stack + IRQ enabled        SchedulerRegistered -> Active
software interrupt usable       Active -> IpiReady
```

The scheduler no longer stores its own `online` or `active` booleans.
Scheduler-local state remains limited to current task, idle task, run queue,
pending switch and IRQ/preemption counters.

A start API error records `Failed`. Timeout diagnostics print every logical
CPU's hardware ID and exact lifecycle state.

Runtime hotplug and degraded boot after a failed secondary remain future work;
they must extend this state machine rather than introduce new masks.
