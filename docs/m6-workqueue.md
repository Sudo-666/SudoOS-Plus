# M6-B workqueue, delayed work, and tickless idle

## Scope

M6-B builds the first deferred-execution layer on the M6-A monotonic clock and
one-shot timer runtime. It provides:

- a bounded per-CPU system workqueue;
- two pinned worker tasks per scheduler-active CPU;
- allocation-free immediate queueing from task or IRQ context;
- delayed work whose timer callback only publishes work;
- synchronous cancellation and flush from sleepable task context;
- directed SMP queueing;
- scheduler-tick suppression while a CPU is truly idle; and
- one-shot wakeup for the earliest software timer or delayed work item.

This is intentionally a small-kernel subset of Linux workqueues. It establishes
the execution-context and lifetime contracts needed by later block-I/O, driver,
and filesystem work without prematurely implementing Linux's full unbound-pool,
rescuer, affinity-scope, CPU-hotplug, or workqueue-attribute machinery.

## Execution contexts

| Operation | Task | Hard IRQ | May sleep |
|---|---:|---:|---:|
| `queue` / `queue_on` | yes | yes | no |
| delayed timer publication | no | yes | no |
| user work callback | yes, system worker | no | yes |
| `flush` | yes | no | yes |
| `cancel_sync` | yes | no | yes |

A delayed timer callback performs only the state transition
`Delayed -> Pending`, increments the lock-free waiter predicate, and wakes one
worker. The user callback never executes from the timer interrupt.

## Per-CPU topology and bounds

Each active CPU owns:

- 128 fixed work slots;
- a FIFO pending ring;
- two pinned `SystemThread` workers;
- one wait queue for worker wakeup; and
- one wait queue plus generation atomics for synchronous completion.

The fixed slots make queueing deterministic and allocation-free after boot.
Two workers guarantee that one callback waiting on an external event does not
immediately stall all deferred work on that CPU. This is a bounded guarantee,
not permission for arbitrary long-lived blocking: subsystems should still keep
work callbacks finite and split long operations into explicit state machines.

## Work state machine

```text
Free
  | queue                         | queue_delayed
  v                               v
Pending                         Arming
  | worker pop                    | timer published
  v                               v
Running                         Delayed
  | callback returns              | timer IRQ
  v                               v
Free <------------------------- Pending

Delayed -- cancel_sync --> Cancelling -- timer quiescent --> Free
Pending -- cancel_sync -----------------------------------> Free
Running -- cancel_sync --> wait for callback ------------> Free
```

Every handle includes owner CPU, slot, and generation. A completed-generation
array lets `flush` and `cancel_sync` wait without acquiring the workqueue base
lock from inside the scheduler wait-queue condition. That avoids a hidden
`Scheduler -> WorkQueue` reverse dependency.

## Cancellation contract

`cancel_sync(handle)` returns `true` only when it prevented the user callback
from starting. If the callback is already running, it waits for completion and
returns `false`. For delayed work it first changes the work state to
`Cancelling`, then calls the M6-A timer `cancel_sync`, and reclaims the slot only
after the timer publication callback is quiescent.

`flush` and `cancel_sync` are rejected from a system workqueue worker. This
conservative rule prevents a pinned worker from synchronously waiting for work
that may require the same bounded pool to make progress. A later dependency-
aware workqueue implementation may relax this with explicit workqueue domains.

## Global lock order

M6-B extends the lock graph to:

```text
CrossCpu < Timer < WorkQueue < Scheduler < WaitQueue
         < VM/PageTable < Heap < PageAllocator
```

Important consequences:

- timer hard IRQ may interrupt an IRQ-enabled cross-CPU serializer;
- a timer callback may publish into a workqueue after releasing `timer_base`;
- workqueue wakeup may acquire the scheduler lock only after releasing
  `workqueue_base`; and
- synchronous wait predicates use atomics and never acquire `workqueue_base`
  while the scheduler lock is held.

Changing numeric ranks without preserving these execution-context rules is not
a valid fix.

## Tickless idle protocol

The clockevent remains one-shot at all times. M6-B separates:

- clockevent hardware started/stopped state; and
- scheduler-policy tick active/inactive state.

Idle entry is:

```text
IRQ disable
  -> final scheduler/reaper work recheck
  -> suppress scheduler tick
  -> program earliest software timer, or shut down clockevent
  -> atomic enable-and-wait-for-interrupt
```

If a timer or IPI wakes the CPU and scheduling switches directly from idle to a
normal task, the context-switch tail restores the scheduler tick **after** the
scheduler lock is released and **before** local IRQs are restored. This is
required both for the global `Timer < Scheduler` order and for preserving normal
timeslice preemption after a tickless idle wakeup.

## Real-hardware constraints

- `MIN_CLOCKEVENT_DELTA_NS` remains a conservative architecture-independent
  floor until each timer driver exposes its actual minimum programmable delta.
- `arch::time::shutdown()` must leave IPIs and unrelated interrupt sources
  operational; it may only suppress the local timer source/deadline.
- CPU hotplug is not implemented. Workqueue topology is frozen after SMP
  bring-up and requires every discovered CPU to be scheduler-active.
- The current fixed slot and worker counts are policy constants. Exhaustion is
  explicit (`WorkError::Capacity`) rather than silent allocation or dropping.
