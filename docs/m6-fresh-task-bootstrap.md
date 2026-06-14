# M6 fresh-task bootstrap contract (r3)

The upper guard faults seen at `vmalloc_base + 0x11000` and `+0x47000` map to
the last per-CPU workqueue worker allocation.  The workqueue does **not** own a
private context constructor: its workers follow

```text
workqueue::initialize
  -> task::spawn_system_thread_on
  -> task::spawn_system_thread
  -> Scheduler::spawn
  -> Task::kernel_thread
  -> fresh_task_context
```

The global defect was the bootstrap ABI contract.  A fresh context reserved
only 16 bytes and the architecture trampoline itself modified memory before
entering Rust.  That made the upper guard safety dependent on the first few
instructions of every architecture entry path.

## r3 invariants

1. Each architecture exports `FRESH_TASK_STACK_RESERVE` (currently 512 bytes).
2. Generic stack code places the saved SP exactly that far below the
   end-exclusive upper boundary.
3. `fresh_task_context()` validates the SP stored by the architecture context
   before the task can be enqueued.
4. Fresh trampolines do not adjust or write the stack before calling the Rust
   bootstrap.
5. Idle, kernel, and system threads use the same constructor.
6. Workqueue code is forbidden from allocating a `KernelStack` or constructing
   an architecture `Context` directly.
7. Both guard pages remain unmapped.

The 512-byte reserve is deliberately architecture-owned.  It is large enough
for a future explicit saved-register/bootstrap frame while leaving 15.5 KiB of
the 16 KiB mapped stack available.  A future architecture that needs a larger
entry frame must increase its exported constant; generic task code remains
unchanged.
