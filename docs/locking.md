# Locking

The current kernel deliberately favors coarse locks until the M5 lifetime rules
are explicit. Correctness comes before scalability.

## Current Locks

| Lock | Type | Protects | IRQ rule |
|------|------|----------|----------|
| `TOPOLOGY` | `IrqSpinLock` | logical to hardware CPU mapping | local IRQs disabled |
| `SCHEDULER` | `IrqSpinLock` | task table, run queues, wait state, CPU scheduler state | local IRQs disabled |
| `VMALLOC` | `IrqSpinLock` | kernel virtual reservations | local IRQs disabled |
| `KERNEL_PAGE_TABLE` | `IrqSpinLock` | runtime kernel page table object | local IRQs disabled |
| kernel heap lock | `IrqSpinLock` | slab and large allocation state | local IRQs disabled |
| `PAGE_ALLOCATOR` | `IrqSpinLock` | buddy/refcount metadata | local IRQs disabled |
| `RETIRED_REAPER` | `SpinLock` | serialized retired task draining | interrupts remain enabled |
| `SHOOTDOWN_SERIALIZER` | `SpinLock` | one synchronous TLB shootdown request at a time | interrupts remain enabled |

## Lockdep Order

`IrqSpinLock` instances must be constructed with a static `LockClass`. The
debug kernel records a per-CPU held-lock stack while local interrupts are
disabled and enforces this global rank order:

1. CPU lifecycle / SMP topology
2. scheduler and run queues
3. wait queue state
4. VM and page-table metadata
5. heap
6. page allocator
7. console

Do not acquire an earlier lock while holding a later lock.

Locks in the same rank use their `order` field as a deterministic sub-order.
Recursive acquisition by the same CPU is rejected, and unlock must happen in
strict LIFO order. On panic, the owning CPU prints its current lock stack,
including lock name, rank, order, and hold time in architecture counter cycles.

## Current Risk Areas

- Plain `SpinLock` users outside `IrqSpinLock` do not yet participate in
  lockdep. Keep these short and avoid nesting them under IRQ spinlocks.
- Console output is still not serialized by a ranked console lock, so ordinary
  multi-CPU logs may interleave.

## Verifier Coverage

- `irq_lock::verify()` checks interrupt save/restore, nested ranked locks,
  recursive `try_lock`, lock hold accounting, and IRQ-off accounting.
- SMP scheduler tests exercise concurrent run queue access through the global
  scheduler lock.
- TLB shootdown tests exercise the serializer while remote CPUs acknowledge IPI
  requests.

## Not Yet Proven

- Whole-kernel AB/BA coverage beyond the currently exercised boot and smoke
  paths.
- Lockdep coverage for plain `SpinLock`.
- Console serialization across CPUs.

## IRQ-enabled tracked spin locks

`TrackedSpinLock` is for task-context cross-CPU protocols that must service
interrupts while serialized. Its guard pins migration but keeps IRQs enabled;
only lockdep metadata updates use short IRQ-save windows.

Recursion compares `LockInstanceId`, while rank/order belong to `LockClass`.
This permits distinct locks of the same class to nest without false recursive
lock reports.

Current order:

1. `RETIRED_REAPER` (`CrossCpu/#1`)
2. `SHOOTDOWN_SERIALIZER` (`CrossCpu/#2`)
3. scheduler and later ranks

Do not silence an order panic by changing a rank without auditing the call
chain. A cross-CPU serializer acquired under VM/page-table/heap state is a
real lifetime/order problem.
