# M8-B2 demand-fault gate

M8-B2 turns the M8-A `UserAddressSpace` contract into a deterministic user
fault state machine before the kernel wires it to a hardware page-table root.
This mirrors the Linux split between VMA/fault classification and the lower
level page-table installation/TLB invalidation path.

The invariant is:

```text
trap frame + fault address
    -> find VMA / maybe grow stack
    -> decide map-anonymous, COW, protection, spurious, or SIGSEGV
    -> install page-table entry while holding the mm page-table lock
    -> drop mm locks
    -> issue per-mm TLB invalidate using active_cpus snapshot
```

This stage intentionally remains architecture-neutral.  M8-B3/B4 should attach
these decisions to the private hardware page-table root and the trap path.

## Rules established here

- User faults outside every VMA may grow only a `GROW_DOWN` stack VMA.
- Stack growth is bounded by a guard gap, one-step growth size, and distance to
  the saved user stack pointer.
- Anonymous/heap/stack not-present faults produce an exact page mapping plan.
- COW is classified but not silently resolved before fork/COW exists.
- Protection violations, kernel faults, and invalid user addresses remain fatal.
- Any successful page-table mutation must be followed by an ASID-scoped TLB
  request planned from the same `UserAddressSpace` generation.
