# M8-B1 per-mm TLB / ASID hardware gate

This gate connects the architecture-neutral `UserAddressSpace` contract from
M8-A to the existing synchronous kernel TLB request protocol.  It deliberately
does **not** create a second IPI mailbox or a second ACK wait path.

## Scope

- RISC-V local ASID invalidation uses `SFENCE.VMA` with an explicit ASID.
- LoongArch local ASID invalidation uses register-specified `INVTLB`:
  - op `0x4`: non-global entries matching one ASID;
  - op `0x5`: non-global entries matching one ASID and virtual address.
- `shootdown_user()` snapshots the M8-A `active_cpus` mask through
  `PerMmTlbRequest` and targets exactly those online/IPI-ready CPUs.
- The current CPU only performs a local invalidation when it is in the request
  target mask.
- Remote CPUs continue to use the existing request slot, serializer, mailbox
  bit, timeout diagnostics, and completion mask.
- Long per-mm ranges fall back to an ASID-wide invalidation, not a global TLB
  flush.

## Lock and wait rule

The caller must finish page-table mutation and release its page-table/VMA locks
before calling `shootdown_user()`.  The TLB serializer is acquired with local
interrupts enabled while migration is disabled.  This CPU must remain able to
acknowledge an unrelated incoming shootdown while contending or waiting.

```text
VMA lock -> page-table lock -> publish PTE change -> unlock
    -> advance mm tlb_generation / snapshot active_cpus
    -> shootdown_user(existing serializer + mailbox + ACK)
    -> retire page/table memory
```

## Runtime proof

The debug verifier creates a synthetic non-kernel ASID and publishes the
current CPU plus at most one remote ready CPU in its `active_cpus` mask.  It
requires:

- exactly the selected remote CPU to increment its remote-flush counter;
- every non-selected CPU counter to remain unchanged;
- the shared request slot to return to FREE;
- the M8-A TLB generation to advance and the synthetic mm to become inactive.

Expected serial evidence:

```text
M8-B1 per-mm TLB test:
  ASID-local invalidate : verified
  exact active CPU mask : verified
  shared ACK protocol   : verified
  generation handshake  : verified
```

## Deliberate boundary

This is the first validation gate inside the M8-B integration delivery.  It does
not yet switch private user roots or resolve demand faults.  Those paths depend
on this exact-ASID invalidation primitive and must not be merged before this
gate passes on both architectures with SMP=1 and SMP=4.
