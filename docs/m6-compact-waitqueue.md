# M6-B compact wait queues and kernel-stack safety

## Root cause

The original `WaitQueue` embedded `[WaitEntry; MAX_TASKS]`, and
`ClaimedWaiters` returned another `[Option<TaskId>; MAX_TASKS]` by value.
Consequently every `Completion` was several KiB.  Debug construction such as
`Box::new(BlockingProbe::new())` could materialise multiple large values on a
16-KiB kernel stack.  A single large stack-pointer decrement can skip a one-page
lower guard and fault in the preceding allocation's upper guard; the faulting
address therefore looked like a worker stack even though the overflowing task
was the verifier allocated immediately after all workers.

The observed layout proves this:

- SMP=1: reaper + 2 workers occupy the first three reservations; the verifier is
  fourth and a skipped lower guard lands at `vmalloc_base + 0x11000`.
- SMP=4: 3 secondary idle stacks + reaper + 8 workers occupy twelve
  reservations; the verifier is thirteenth and lands at `+0x47000`.

Changing fresh-context SP headroom cannot change either address because the
problem is a later oversized Rust stack frame.

## Linux-like design

A wait-queue head is constant-size: lock + head/tail/count.  Queue membership is
intrusive in `Task` (`wait_prev`, `wait_next`).  No wake path allocates and no
MAX_TASKS-sized object is copied through the kernel stack.

Global lock order remains:

```
Scheduler -> WaitQueue
```

The scheduler and queue lock jointly protect task state, channel ownership, and
intrusive links.  Timeout-vs-normal-wakeup races still claim a waiter exactly
once.

## Invariants

1. A task belongs to at most one wait queue.
2. `wait_channel == None` implies both intrusive links are clear.
3. A linked task is `SwitchingOut` or `Blocked`.
4. Wakeup unlinks first, then performs the scheduler state transition.
5. `SwitchingOut` wakeup leaves `wait_channel` set until switch-tail but clears
   links immediately and sets `wake_after_switch`.
6. Queue head and `Completion` have compile-time size caps.
7. Guard pages remain unmapped; no stack size or mapping workaround is used.
