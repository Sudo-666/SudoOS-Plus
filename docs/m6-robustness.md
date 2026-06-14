# M6 robustness review

## Problems closed by M6-A/M6-B and r1-r5

- Timer and work slots use generations, preventing stale-handle aliasing after
  reclamation.
- Synchronous cancellation covers the armed-versus-firing race.
- Timer hardirq work is bounded; delayed sleepable callbacks execute in system
  worker task context.
- Retired task destruction is separated from scheduler ownership and stack
  lifetime.
- Fresh task entry uses an architecture-owned mapped stack reserve instead of
  relying on an end-exclusive guard boundary.
- WaitQueue storage is intrusive and O(1) per queue rather than embedding
  arrays sized to the global task limit.
- The lock graph includes CrossCpu, Timer, WorkQueue and Scheduler ordering.
- Tickless idle has one owner for stop and one owner for restart; verifier code
  no longer creates a second timer state machine.
- Remote idle wake is verified with a single measured reschedule IPI.

## Problems closed by M6-C

- A milestone is tied to an exact Git commit rather than a remembered terminal
  session.
- Debug-only success is no longer enough: Release, SMP=2/8 and multiple memory
  sizes are part of the full gate.
- Repeated SMP=4 evidence is retained in structured logs and JSON.
- Source hygiene rejects tracked Python caches, build output and editor/macOS
  artifacts.
- A static cross-file audit catches regression of stack, wait-queue, lock-order,
  NO_HZ and smoke-evidence contracts before QEMU starts.
- The fixed-capacity resource boundary is explicit: queueing functions must be
  fallible, and callers must handle exhaustion.

## Remaining boundaries before drivers

The current fixed two-workers-per-CPU design is suitable for M6 deferred
execution and deterministic testing. It is not yet a general Linux workqueue
replacement.

Before M7/M15 code can block while waiting for work, choose one of these:

1. a dedicated workqueue for the subsystem with a no-self-dependency rule;
2. an unbound pool with worker growth;
3. a reserved rescuer worker and emergency work slots.

Until then, callbacks submitted to the shared M6 workqueue must not wait for
other work on that same queue. Resource exhaustion must propagate to the caller
instead of panicking or silently dropping work.

## Physical-machine boundary

QEMU proves architecture code paths and many concurrency invariants, but not
real interrupt-controller quirks, non-coherent DMA, MMIO ordering, timer drift,
firmware defects or cache topology. Hardware validation belongs to the platform
and driver milestones and must not be inferred from the M6 tag.
