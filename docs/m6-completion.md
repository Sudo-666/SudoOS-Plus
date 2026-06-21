# M6-C completion contract

M6 is frozen only after the exact source commit passes the local release gate.
A successful ad-hoc QEMU run is useful evidence, but it is not a release record.

## Frozen scope

M6 provides:

- monotonic clock and architecture clockevent integration;
- bounded per-CPU timer queues with generation-stamped handles;
- synchronous timer cancellation;
- kernel sleep, WaitQueue timeout and Completion timeout;
- bounded hardirq callback dispatch;
- per-CPU workqueue workers;
- immediate work, delayed work, flush and synchronous cancellation;
- generation-stamped work handles and slot reclamation;
- tickless idle with idle-path ownership of tick stop and switch-tail ownership
  of tick restart;
- deterministic exact-one-IPI idle wake verification;
- compact intrusive wait queues and architecture-owned fresh-task stack entry.

## Required invariants

1. Hardirq code does not sleep and does not execute sleepable work callbacks.
2. Timer and work callbacks run without their base lock held.
3. A stale generation handle cannot refer to a newly reused slot.
4. `cancel_sync` does not return while the selected callback is still running.
5. Idle tick stop occurs only through `time::enter_idle()`.
6. A normal task cannot execute after idle until `time::leave_idle()` has run.
7. Tick restoration happens after releasing the Scheduler lock.
8. WaitQueue heads remain O(1)-sized; waiter links belong to Task.
9. Fresh tasks start from an architecture-validated mapped stack pointer.
10. Fixed-capacity timer/work APIs report exhaustion through `Option` or
    `Result`; callers must not assume queueing is infallible.

`scripts/m6-audit.py` enforces the cross-file parts of this contract.

## Release gates

### Quick developer gate

```bash
make m6-quick
```

Runs the static M6 audit, the normal `make check` gate, and debug smoke for both
architectures at SMP=1 and SMP=4 with 256 MiB.

### Full compatibility matrix

```bash
make m6-full
```

Runs both architectures with SMP=1/2/4/8, memory=64 MiB/256 MiB/1 GiB, and
Debug/Release profiles.

### Soak only

```bash
M6_SOAK_LOOPS=50 M6_RELEASE_SOAK_LOOPS=10 make m6-soak
```

Runs repeated dual-architecture SMP=4 Debug and Release smoke. Increase the
loop counts before a major refactor or before first hardware bring-up.

### Release gate

```bash
make m6-release
```

Requires a clean Git worktree and runs the full matrix plus soak. Evidence is
saved under `build/m6/<UTC>-<commit>-release/` and records the exact commit,
commands, logs, durations and result.

Only after this passes may the milestone tag be created:

```bash
make m6-tag
```

## Deliberate limits

M6 does not claim:

- an unbound workqueue pool or dynamic worker creation;
- a memory-reclaim rescuer worker;
- CPU hotplug;
- hard real-time latency guarantees;
- production validation on physical RISC-V or LoongArch machines;
- device DMA, external interrupt routing or interrupt affinity.

These omissions do not invalidate the M6 kernel foundation. They become hard
requirements before block I/O, filesystem reclaim or other callbacks are
allowed to synchronously wait for work queued to the same bounded pool.

## Change policy after freeze

Any later modification to timer, workqueue, WaitQueue, task-switch tail,
clockevent policy, lock ranks or idle entry must run at least `make m6-full`.
Changes to cancellation, idle/IPI, stack bootstrap or lock ordering require
`make m6-release` before merging into the frozen baseline.
