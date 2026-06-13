# CPU Lifecycle

The current lifecycle is implemented with topology data plus online, IPI-ready
and scheduler-active masks. M5 PR 8 will turn this into a single explicit state
machine.

## Current States

```text
Discovered
  -> Starting
  -> Online
  -> SchedulerRegistered
  -> SchedulerActive
  -> IpiReady
```

The implementation currently publishes `SchedulerActive` before `IpiReady`, but
CPU 0 waits for both before leaving bring-up. Normal work therefore cannot target
a secondary CPU until it can also receive the wakeup kick.

## State Responsibilities

| State | Owner | Meaning |
|-------|-------|---------|
| Discovered | CPU 0 | FDT exposed a hardware CPU ID and the logical mapping is known. |
| Starting | CPU 0 | Architecture start protocol or mailbox has been issued. |
| Online | secondary | The CPU reached Rust, installed trap/timer/VM state and registered with SMP. |
| SchedulerRegistered | secondary | The scheduler owns a permanent idle context for the CPU. |
| SchedulerActive | secondary | The idle task is running with local interrupts enabled. |
| IpiReady | same CPU | The CPU can receive software interrupts used for reschedule/TLB work. |

## Current Validation

- CPU 0 waits for `online_cpu_count() == discovered_cpu_count()`.
- CPU 0 waits for the expected IPI-ready mask.
- `task::finalize_cpu_bringup()` checks scheduler online/active state against
  SMP online/IPI-ready masks.
- Smoke tests require discovered, online and participating CPU counts to match
  the QEMU `SMP` setting.

## Not Yet Supported

- CPU hotplug.
- Partial startup rollback.
- Disabling a discovered CPU and continuing with a smaller active mask.
- Panic stop IPI and frozen CPU confirmation mask.

## Immutable CPU identity map

Firmware discovery constructs the logical-to-hardware CPU identity map once on
the boot CPU. The map is immutable after publication:

1. hardware-ID entries are stored;
2. the discovered count is published with `Release`;
3. readers load the count with `Acquire`;
4. IPI and TLB paths read the corresponding hardware ID without taking a lock.

CPU lifecycle state (`online`, interrupt-ready, scheduler-active, dying, dead)
is separate from CPU identity. Future hotplug transitions may change lifecycle
state, but they must not rewrite the logical-to-hardware identity map.

This separation prevents runtime cross-CPU paths from acquiring the
`CpuLifecycle` lock while holding a TLB, scheduler, VM, or other later-ranked
lock.
