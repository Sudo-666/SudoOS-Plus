# M8 Linux-like user-MM contract

This document defines the accepted M8 architecture for SudoOS. It deliberately
stops at the boundary before schedulable user processes.

## Ownership model

During M8, one synchronous verifier session owns one address space:

```text
UserImage
  └── Box<UserMm>
        ├── private lower user page-table root
        ├── shared kernel mappings
        ├── ASID token and generation
        ├── VMA metadata
        ├── mapped backing pages
        └── active_cpus / TLB generation state
```

`ACTIVE_MM` is a temporary verifier-session binding. It is not a scheduler
current-mm facility and must not be replaced by a per-CPU raw-pointer array.
The owning `Box<UserMm>` remains alive for the complete bind/activate/user
round-trip/deactivate/unbind/destroy sequence.

M9 will introduce the production ownership chain:

```text
Task -> Process -> Arc<UserMm>
```

and separate that ownership from per-CPU loaded-MMU state.

## M8/M9 boundary

M8 includes:

- an independent user page-table root;
- shared high-half kernel mappings;
- architecture ASID allocation and generation rollover;
- `mm.active_cpus` publication protocol;
- exact per-mm TLB request planning and remote-ACK infrastructure;
- a synchronous single-CPU user execution gate;
- anonymous demand paging and bounded stack growth;
- `brk`, private anonymous `mmap`, `munmap`, and `mprotect`;
- checked user copies and fault retry;
- TLB-before-free retirement ordering.

M8 does not include:

- Process/Thread ownership of `UserMm`;
- scheduler-driven `switch_mm_irqs_off()`;
- concurrent user sessions;
- migration of a live user task;
- shared-mm user threads;
- signals, fork, COW, ELF, VFS, or file-backed mappings.

Those belong to M9 and later milestones.

## Hardware-switch invariant

The M8 verifier disables local interrupts for the full private-root round trip:

```text
bind verifier session
  -> disable local interrupts
  -> install private root + ASID
  -> synchronize stale local ASID state
  -> publish current CPU in mm.active_cpus
  -> execute PLV3/U-mode
  -> restore kernel root + kernel ASID
  -> flush the departing user ASID locally
  -> remove current CPU from mm.active_cpus
  -> restore local interrupt state
  -> unbind verifier session
```

The active mask is expected to contain exactly one CPU during this gate.
That assertion is intentional: it detects leaked publication bits and does not
claim that a schedulable shared-mm implementation already exists.

This mirrors Linux's ordering principles, while deferring Linux's
task-owned `mm` and scheduler `switch_mm_irqs_off()` mechanism to M9.

## ASID invariant

- ASID zero is reserved for the kernel.
- A `UserMm` owns an ASID token with an allocator generation.
- Reuse after generation rollover requires the architecture-wide stale ASID
  state to be invalidated before the new generation is published.
- M8 allows rollover only when no other live user MM exists. This is a strict,
  simple policy appropriate to the single-session verifier.
- M9 may replace that restriction with per-CPU generation tracking similar to
  Linux once tasks can own and switch address spaces.

## active_cpus and TLB invariant

`UserAddressSpace.active_cpus` describes CPUs that have completed installing
this MM's root/ASID and have not completed departure. Entry and exit use a
check/publish handshake so an invalidation cannot miss a CPU racing with an
address-space switch.

There are two explicit execution paths:

- `shootdown_user(request)` is remote-capable, pins migration, sends exact
  target IPIs, waits for ACKs, and requires interruptible task context.
- `shootdown_user_local(request)` is used by the M8 IRQ-off synchronous user
  gate and fails closed if the request unexpectedly contains another CPU.

The path is selected by the execution model, not by an ad-hoc runtime test of
the interrupt-enable bit.

## Fault invariant

For a recoverable anonymous, heap, or stack fault:

```text
decode fault
  -> validate VMA and access
  -> allocate a zeroed page
  -> lock and revalidate
  -> install PTE
  -> release MM/page-table lock
  -> invalidate exact ASID/page
  -> retry the original user instruction
```

Protection and unmapped faults terminate only the verifier session. Kernel
faults remain fail-fast.

No allocator, remote wait, or page free is allowed while holding the MM's
page-table/VMA lock.

## Retirement invariant

Unmapping follows the Linux `mmu_gather` ordering even though M8 uses a compact
implementation:

```text
detach PTEs under the MM lock
  -> collect backing and page-table pages
  -> release the MM lock
  -> complete the required TLB invalidation
  -> wait for all required acknowledgements
  -> free backing pages
  -> free empty page-table pages
```

`finish_retirement()` must contain the flush-before-free ordering directly.
Future M9 work may turn this into an explicit `MmuGather` object.

## LoongArch entry and TLB invariants

- `$r21/u0` is user-controlled on entry but kernel-owned at PLV0 as the logical
  CPU identifier. Trap entry saves the user GPR value, then reloads the kernel
  value from KSave3/CSR `0x33` before calling Rust.
- User payload code after `__m8_user_vm` must not use `$r21`.
- A demand-fault refill can cache a global invalid leaf pair. Page-local user
  invalidation therefore uses INVTLB operation `0x6`, which removes either a
  global entry at the pair address or the matching non-global ASID entry.
  Operation `0x5` is insufficient for this case.

## Required gates

```sh
python3 scripts/m8-audit.py
python3 scripts/m8b3-audit.py
python3 scripts/m8b4-audit.py
python3 scripts/m8-linuxlike-audit.py
make fmt-check
make check
make verify
```

The final runtime matrix is:

```sh
SMP=1 PROFILE=debug   SMOKE_TIMEOUT=240     make smoke-loongarch64
SMP=4 PROFILE=debug   SMP_SMOKE_TIMEOUT=300 make smoke-smp-loongarch64
SMP=1 PROFILE=release SMOKE_TIMEOUT=240     make smoke-loongarch64
SMP=4 PROFILE=release SMP_SMOKE_TIMEOUT=300 make smoke-smp-loongarch64

SMP=1 PROFILE=debug   SMOKE_TIMEOUT=240     make smoke-riscv64
SMP=4 PROFILE=debug   SMP_SMOKE_TIMEOUT=300 make smoke-smp-riscv64
SMP=1 PROFILE=release SMOKE_TIMEOUT=240     make smoke-riscv64
SMP=4 PROFILE=release SMP_SMOKE_TIMEOUT=300 make smoke-smp-riscv64
```

Freeze M8 only when all eight pass with `demand fault path : verified`, with no
panic, timeout, lockdep report, kernel page fault, stale translation, or
MM/root/backing leak. Then run at least 50 debug SMP=4 iterations on each
architecture with zero flaky classifications.
