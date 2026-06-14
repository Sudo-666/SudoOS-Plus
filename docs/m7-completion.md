# M7-B completion contract

M7 is frozen only after the exact source commit passes the local release gate.
A successful hello-user boot is necessary but not sufficient: the same gate must
prove negative syscall behavior, checked user copies, user protection faults,
trap-stack restoration and repeated mapping teardown.

## Frozen scope

M7 provides:

- RISC-V U-mode and LoongArch PLV3 synchronous entry;
- user trap entry on the current task's kernel stack;
- Linux generic 64-bit syscall numbers `write=64` and `exit=93`;
- architecture ABI register extraction and return values;
- one RX user code page, one RW data page and one RW stack page;
- checked `copy_from_user` and `copy_to_user`;
- `-ENOSYS`, `-EBADF`, `-EINVAL` and `-EFAULT` error paths;
- user page-fault and exception termination without converting ordinary user
  failure into a kernel fault;
- synchronous return from `sys_exit` to the original kernel call stack;
- complete unmap, TLB shootdown, empty-table reclamation and backing-page free;
- Debug and Release smoke evidence on both architectures.

## Required runtime evidence

Each boot runs five isolated user sessions:

1. valid `write(1, "hello user\n", 11)` followed by `exit(0)`;
2. unknown syscall, verifying `-ENOSYS`;
3. `write` with an unmapped pointer, verifying checked-copy `-EFAULT`;
4. a store to the RX code page, verifying a user protection fault;
5. another valid write/exit after the fault, proving teardown and reuse.

The verifier also rejects:

- kernel writes through the RX user mapping;
- cross-page user copies;
- overflowing user ranges;
- access after the session was unpublished and unmapped.

## Deliberate limits

M7 does not claim:

- a per-process page-table root;
- ASIDs or per-mm TLB shootdown;
- preemptible user threads;
- demand paging or stack growth;
- ELF loading;
- a general syscall table;
- signals;
- VFS or file descriptors beyond the provisional console `fd=1`;
- physical-machine validation.

The current user round trip disables local interrupts. This keeps the active
kernel stack owned by one kernel task until M9 introduces schedulable user
threads. M8 replaces the temporary mappings in the kernel root with a real
per-process `AddressSpace`.

## Release gates

```bash
make m7-audit
make m7-quick
make m7-full
M7_SOAK_LOOPS=50 M7_RELEASE_SOAK_LOOPS=10 make m7-soak
make m7-release
make m7-tag
```

`m7-release` requires a clean worktree and records the exact commit, command
matrix, logs and results under `build/m7/`.
