# M8-A: user-MM core contract

This delivery establishes the architecture-neutral half of SudoOS M8 without
changing either architecture's active page-table root yet.

## Why this boundary exists

M7 maps three user pages into the shared kernel runtime page table and performs
kernel-wide TLB invalidation. Reusing that ownership model for `mmap`, demand
paging, or process migration would make every later process share one root and
would turn ordinary user invalidations into global kernel shootdowns.

M8-A therefore introduces the state that must exist before hardware switching:

- generation-tagged ASIDs, with ASID 0 reserved for the kernel;
- an ASID cursor one bit wider than hardware, so maximum ASID 65535 never wraps
  through reserved ASID 0;
- an atomic `active_cpus` mask equivalent to Linux `mm_cpumask(mm)`;
- a generation handshake that closes the CPU-reentry-versus-shootdown race;
- per-mm TLB request planning from an `active_cpus` snapshot;
- transactional exact-range VMA permission replacement;
- bounded `VM_GROWSDOWN` planning using a preceding-VMA guard gap, a maximum
  growth step, and the saved user SP;
- destruction refusal while an address space is active on any CPU;
- host tests for rollover, stale-ASID rejection, CPU membership, TLB scope,
  reentry races, stack growth, guard gaps, rollback, and existing anonymous-
  fault classification.

## Locking and lifetime rules

The kernel-side M8-B wrapper will own the page-table root and locks. The legal
order is:

```text
VMA metadata lock -> page-table lock
```

A synchronous remote TLB wait is never performed while either lock is held.
The mutating path records the invalidation, releases locks, advances the
per-mm TLB generation, snapshots `active_cpus`, sends IPIs, waits for
acknowledgements, and only then frees retired data/table pages. This follows
the same lifetime principle as Linux's `mmu_gather`/TLB teardown path.

The two atomics participating in the reentry handshake deliberately use
sequentially consistent operations in M8-A. This gives one reviewable total
order before architecture code exists. M8-B may weaken barriers only with a
written proof for both RISC-V and LoongArch.

CPU entry ordering is:

```text
validate allocator ASID generation
install hardware root + ASID
flush/synchronize local ASID to mm.tlb_generation
publish CPU in active_cpus
re-read mm.tlb_generation
if changed: remove CPU, flush, retry
return to user
```

Insertion precedes the final generation check. Therefore a racing invalidation
either observes the CPU in `active_cpus`, or changes the generation and forces
the entry path to retry. This prevents a CPU that retained an old ASID-tagged
translation from re-entering after being omitted from an earlier shootdown.

CPU exit ordering is:

```text
install the next hardware root
invalidate the departed ASID locally
verify the flushed mm.tlb_generation is still current
remove CPU from active_cpus
```

A generation mismatch leaves the CPU published, so teardown cannot silently
escape a concurrent shootdown. A stale allocator ASID token is rejected and
must be refreshed after the allocator's generation-wide flush.

## Stack growth policy

A missing page below a stack VMA is not automatically accepted. It must be:

- below a `Stack + GROW_DOWN` VMA;
- associated with a saved SP inside that VMA (or exactly at its top boundary);
- within one configured growth step;
- close enough to the trap frame's saved user SP;
- separated from the preceding VMA by the configured stack guard gap;
- fully inside the user address range.

This prevents a random low-address fault from expanding the stack across a
large hole or into a neighboring mapping.

## Deliberately not done in M8-A

- allocating/cloning a per-process hardware root;
- sharing only the kernel high-half root entries;
- RISC-V `satp` and LoongArch PGDL/ASID switching;
- installing anonymous pages into a private root;
- wiring fault recovery and user-copy fault fixups into trap entry;
- remote per-mm IPI execution and page retirement.

Those are one coupled hardware integration and closure delivery (M8-B). M8-A
must pass host tests first so architecture failures are not mixed with
VMA/ASID state-machine errors.
