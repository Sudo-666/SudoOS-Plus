# Boot Order

This document records the current initialization contract. Keep it in sync with
`kernel/src/main.rs` whenever a subsystem moves earlier or later in boot.

## Common Order

1. Architecture entry code establishes a high-half kernel execution context.
2. `rust_entry()` installs logical CPU 0 in the architecture per-CPU register.
3. `kernel_main()` parses the firmware device tree and initializes SMP topology.
4. Boot memory layout is built from RAM, reserved regions, kernel image and FDT.
5. Early memory constructs boot page tables and architecture-specific mappings.
6. The global page allocator is installed, then heap allocation is enabled.
7. Trap, IRQ, timer, kernel VM and page fault subsystems are initialized.
8. The periodic timer starts before the task scheduler is initialized.
9. The scheduler creates boot and secondary idle tasks.
10. Secondary CPUs are started and must reach online and IPI-ready states.
11. The scheduler verifies that every online CPU is scheduler-active.
12. Debug verifiers run before `SMOKE_TEST: PASS`.
13. CPU 0 enters the boot idle loop.

## Important Ordering Constraints

- Heap initialization requires the global page allocator.
- Kernel VM initialization requires the runtime page table handoff.
- Page fault handling must be installed after traps and kernel VM exist.
- Secondary CPUs must not be targeted by scheduler work before they are both
  scheduler-active and IPI-ready.
- Task stack reclamation may perform synchronous TLB shootdown, so it must run
  only with local interrupts enabled in task/idle context.

## Current Validation

- Debug boot verifies page allocator, heap, IRQ spin locks, VM lifecycle, fault
  counters, trap frame preservation, periodic timer, scheduler, wait queues,
  task migration and kernel-wide TLB shootdown.
- `scripts/smoke.py` requires subsystem evidence before accepting
  `SMOKE_TEST: PASS`.

## Not Yet Proven

- User-mode entry and syscall return.
- External device IRQ delivery.
- CPU hotplug or recovery after a partially failed secondary startup.
- Per-address-space ASID/range TLB shootdown.
