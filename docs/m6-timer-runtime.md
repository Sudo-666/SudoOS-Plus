# M6-A Timer and Timeout Runtime

M6 is split into two large deliveries rather than many small patches:

1. **M6-A** — monotonic time, one-shot clockevents, software timers, sleep,
   wait/completion timeouts, and scheduler switch-commit correctness.
2. **M6-B** — workqueues, delayed work, deferred execution, true tickless idle,
   SMP stress/fault injection, and the M6 release gate.

This file documents M6-A.

## Design boundary

The architecture layer remains responsible only for:

- reading a stable monotonic counter;
- ensuring all online CPUs observe one coherent monotonic domain, or applying
  architecture-specific per-CPU offset normalization before exposing it;
- reporting its frequency;
- programming an absolute one-shot deadline;
- acknowledging and masking/unmasking the local timer interrupt.

The common kernel owns policy:

- a typed wrapping `MonotonicInstant`;
- scheduler-policy deadlines;
- per-CPU software timer queues;
- timeout and sleep semantics;
- choosing the earliest local hardware deadline.

This preserves the clocksource/clockevent separation used by mature kernels and
keeps QEMU-specific timer details out of generic code. A real board only needs a
correct architecture timer backend; the common runtime does not assume a QEMU
frequency or a permanently periodic interrupt.

## Per-CPU timer bases

Each CPU owns a fixed-capacity base containing 128 timer slots and a
sorted deadline queue. A timer handle records:

```text
owner CPU + slot index + generation
```

Reusing a slot increments its generation. A stale handle therefore cannot
cancel an unrelated timer that later occupies the same slot.

The initial implementation uses a bounded sorted array instead of heap
allocation in IRQ context. It is intentionally simple and auditable. A heap or
hierarchical wheel can replace the queue later without changing the public
lifetime rules.

## Lifecycle and synchronous cancellation

```text
Free -> Armed -> Firing -> Free
          |
          +-------------> Free (cancelled)
```

Callbacks execute after releasing the timer-base lock. `cancel_sync()` removes
an armed timer or waits until a firing callback has completely returned. This
is the same lifetime property required by Linux-style synchronous timer
cancellation: callers may place callback context on a kernel stack only because
they synchronise before that stack object leaves scope.

Rules:

- `cancel_sync()` requires sleepable task context and local interrupts enabled;
- a callback must never synchronously cancel itself;
- callbacks run in hard IRQ context and must not sleep;
- callback work is bounded to 32 expirations per interrupt;
- if more timers are due, the clockevent is reprogrammed immediately and work
  continues in a later interrupt.

## Clockevent programming

The hardware timer is always programmed one-shot for the earlier of:

```text
next scheduler-policy deadline
next local software-timer deadline
```

A software timer expiring before the next scheduler deadline does not consume a
timeslice. If several scheduler periods elapsed while interrupts were delayed,
the scheduler receives the elapsed tick count rather than one fabricated tick.

Publishing a new local earliest timer and programming the local clockevent are
one IRQ-atomic transaction. This prevents an interrupt or callback from
installing a stale later deadline over a newly inserted earlier timer. Starting
a CPU clockevent also consults an already-populated software queue.

The common layer currently applies a conservative 1 microsecond minimum delta.
A future platform contract should expose each real clockevent device's exact
`min_delta`/`max_delta`; until then the common runtime never assumes that a real
device can accept a deadline only one counter cycle ahead.

M6-A intentionally retains the existing 100 Hz scheduler policy so M5 behavior
remains comparable. The hardware is nevertheless one-shot. Removing the
scheduler deadline while a CPU is idle is M6-B work.

## Sleep and timeout APIs

M6-A adds:

```rust
timer::sleep(duration)
timer::sleep_until(deadline)

WaitQueue::wait_timeout(duration, condition)
WaitQueue::wait_until_deadline(deadline, condition)
Completion::wait_timeout(duration)
```

A wait timeout uses a stack-resident timeout context and a targeted waiter
claim. Normal wakeup and expiry race through one atomic state. After wakeup the
waiter performs synchronous cancellation and checks the protected condition one
final time. A condition observed true on the expiry boundary wins and reports
success rather than a spurious timeout.

No timeout callback broadcasts to the full queue.

## Lock order

M6-A adds the timer-base rank below cross-CPU, scheduler, and wait-queue locks:

```text
Timer(10) < CrossCpu(15) < Scheduler(20) < WaitQueue(30)
```

Timer callbacks run with the timer lock released. This is essential because a
callback may wake a task and therefore acquire scheduler and wait-queue state.

## Fresh-context and switch-commit prerequisite

M6 timeout tests block fresh kernel threads and therefore exercise the first
context switch more aggressively than M5. The patch fixes two scheduler
invariants instead of masking failures by mapping a guard page:

1. a fresh context first enters an architecture assembly trampoline, which
   creates a mapped 16-byte ABI caller frame before calling Rust;
2. `cpu.current` is committed only after the hardware stack has changed and the
   incoming task executes `finish_switch()`.

The pending switch records both `previous` and `next`. Debug builds verify that
the actual hardware stack pointer belongs to `next` before publishing it as the
current task. Kernel stack guard pages remain unmapped.

## Debug verification

Expected M6-A marker:

```text
timer runtime test:
  deadline ordering : verified
  software deadline : verified
  synchronous cancel: verified
  kernel sleep      : verified
  wait timeout      : verified
  slot reclamation  : verified
```

The verifier itself runs in a normal kernel thread. The boot CPU idle task only
yields while it runs; idle tasks are never weakened to allow blocking.

## Validation matrix

Run after applying and formatting:

```bash
make check

make smoke-riscv64 SMP=1 SMOKE_TIMEOUT=240
make smoke-loongarch64 SMP=1 SMOKE_TIMEOUT=300
make smoke-riscv64 SMP=4 SMOKE_TIMEOUT=240
make smoke-loongarch64 SMP=4 SMOKE_TIMEOUT=300

make build-riscv64 PROFILE=release
make build-loongarch64 PROFILE=release
make m5-quick
```

Do not merge on only one architecture. Inspect both `smoke.log` files if a run
fails, and keep the guard-page and two-phase switch assertions enabled while
finding the root cause.

## M6-B remains

M6-A is not the M6 completion tag. M6-B still needs:

- per-CPU worker threads and an unbound fallback;
- immediate work and delayed work state machines;
- synchronous cancel/flush and safe requeue rules;
- a deferred/bottom-half layer for callbacks too heavy for hard IRQ context;
- true tickless idle;
- platform clockevent `min_delta`/`max_delta` contracts and real-board timer
  validation;
- cross-CPU clocksource-coherency validation or per-CPU offset calibration;
- concurrent multi-CPU timer/work stress, cancellation races, fault injection,
  soak tests, and a guarded M6 release/tag workflow.
