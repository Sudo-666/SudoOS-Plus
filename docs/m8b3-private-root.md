# M8-B3 private user-root hardware gate

## Scope

M8-B3 connects the architecture-neutral M8-A/B2 MM contracts to real hardware
page-table roots without yet enabling recoverable demand faults.

The gate deliberately keeps the M7 verifier synchronous and non-preemptible:

1. build a `UserMm` with a private root and generation-tagged ASID;
2. explicitly prepopulate the M7 code, data, and stack pages;
3. install the private lower/user root with local interrupts disabled;
4. invalidate the local ASID and publish the CPU in `active_cpus`;
5. enter user mode and service traps through the shared kernel mapping;
6. restore the permanent kernel root, invalidate the departing ASID, and only
   then clear the CPU from `active_cpus`;
7. unmap leaves, reclaim private intermediate tables, free backing pages, and
   finally release the owned root.

The B2 `UserFaultPlan` remains in `myos-mm`, but the trap path still terminates
user faults with `-EFAULT`. This isolates hardware-root failures from anonymous
allocation, stack expansion, and instruction retry behavior.

## Linux-like ownership model

`UserAddressSpace` is the architecture-neutral `mm_struct` core. The kernel
wrapper owns the hardware page-table root, backing pages, and page-table lock.
The ASID allocator uses generations and reserves ASID zero for the permanent
kernel address space.

The M8 verifier fails closed at ASID rollover while an old-generation MM is
alive. A separate atomic publication gate blocks concurrent allocations while
the global rollover flush runs; the Vm-ranked allocator lock is released before
entering the lower CrossCpu-ranked shootdown serializer. M9 may replace this
with Linux-style lazy ASID renewal once address spaces belong to schedulable
process/thread objects.

### RISC-V

Each user root owns its low-half tables and copies only the high-half Sv39 root
entries from the permanent kernel root. Descendant kernel tables remain
kernel-owned and shared.

Because copied root entries borrow those descendants, kernel high-half empty
intermediate tables are pinned while any user root exists. This prevents an
active or dormant user PGD from retaining a dangling pointer if a kernel
mapping is removed. User roots are forbidden from mutating high-half entries.

### LoongArch

The private user root is installed only in PGDL. PGDH permanently names the
kernel root, so kernel mappings are shared by hardware rather than copied into
the user root. ASID and PGDL are changed together while migration and local
interrupts are disabled.

## Ordering and locking

The switch-in order is:

```text
ASID-generation lock
  -> user-mm lock
  -> synchronize shared kernel root state
  -> install hardware root + ASID
  -> local ASID flush
  -> active_cpus insert
  -> final TLB-generation check
```

The switch-out order is:

```text
install permanent kernel root + ASID 0
  -> local departing-ASID flush
  -> verify TLB generation
  -> active_cpus remove
```

No remote TLB ACK is awaited while holding the user-mm lock. M8-B1 remains the
only cross-CPU shootdown/ACK protocol.

## Runtime evidence

Debug smoke prints:

```text
M8-B3 private-root gate:
  private user root : verified
  kernel high half  : shared
  ASID root switch  : verified
  active CPU publish: verified
  kernel root return: verified
  page/root reclaim : verified
  demand fault path : intentionally deferred
```

All frozen M7 evidence remains unchanged, including five session-recycle runs.

## Deferred work

The next gate wires the B2 planner into the real trap path and adds transactional
anonymous page installation, bounded stack growth, instruction retry, and
per-mm post-install invalidation. `brk`, `mmap`, `munmap`, and `mprotect` remain
part of the final M8 integration/closure work.
