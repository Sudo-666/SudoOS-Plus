# M6 fresh-task stack contract

M6-B r2 fixes a global task-bootstrap invariant rather than special-casing the
workqueue.

## Invariant

For every newly created idle task, counted kernel thread, and permanent system
thread:

1. the saved context SP points **inside** the mapped usable stack;
2. it never equals the end-exclusive `VirtRange::end()` address;
3. both lower and upper guard pages remain unmapped;
4. the architecture trampoline may reserve an additional ABI frame only below
   that already-valid SP;
5. task creation has one factory: `fresh_task_context()`.

`KernelStack::top()` is deliberately removed from the task-facing API.  The
only exported bootstrap operation is `initial_stack_pointer()`, which reserves
aligned headroom and checks that the result belongs to the usable mapping.

## Why this is global

`TaskKind::KernelThread` and `TaskKind::SystemThread` share
`Task::kernel_thread()`.  Workqueue workers, reapers, verifier threads, future
I/O workers, and other kthreads therefore receive the same bootstrap contract.
No subsystem may invent its own guard-adjacent initial SP.

## Guard-page policy

The fix does not map either guard page and does not enlarge the 16 KiB stack.
A true downward stack overflow still faults on the lower guard.  The upper guard
continues to catch invalid upward accesses, but a fresh context is no longer
published with SP equal to that guard's first address.
