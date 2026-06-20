# M10 completion: ELF64 loader, initramfs, and initial exec path

M10 replaces the M7-M9 raw user-code bootstrap with a Linux-like first exec
path while preserving the verified Process/Thread/UserMm ownership model.

## What is implemented

- A strict `newc` initramfs parser with bounded cursor arithmetic, path lookup,
  trailer validation, 4-byte padding handling, and malformed-archive rejection.
- A strict ELF64 little-endian executable loader for the active architecture
  (`EM_RISCV` or `EM_LOONGARCH`), currently accepting `ET_EXEC` and `PT_LOAD`.
- Segment validation for `p_filesz <= p_memsz`, file bounds, user-range bounds,
  power-of-two alignment, `p_offset % p_align == p_vaddr % p_align`, and W^X.
- Page-backed loading through `UserMm::populate_page()`, so RX text is filled by
  the kernel loader without weakening user copy permission checks.
- A Linux initial stack containing `argc`, `argv[0]`, null `envp`, and auxv
  entries for `AT_PAGESZ`, `AT_ENTRY`, and `AT_NULL`.
- A `kernel_execve_from_initramfs()` entry that opens `/init` from initramfs,
  loads the ELF, creates the initial `Process` and `Thread`, and publishes a
  real initial user stack pointer to the scheduler.

## Deliberate boundary

This is the first-exec/kernel-exec half of M10. It does not yet implement
in-place userspace `execve(2)` replacement of a running process image. That
requires the next VFS/fd-table work and a controlled replacement path for the
currently immutable `Process -> Arc<UserMm>` ownership established in M9.

The current boundary is intentionally Linux-like: early boot can execute an
initramfs `/init`, while full user-triggered `execve(2)` becomes a small
extension once VFS path lookup and process image replacement exist.

## Closure gate

M10 is covered by the existing user-mode verifier, now running every test case
through:

```text
generated ELF64 /init -> newc initramfs -> kernel_execve_from_initramfs()
    -> ELF PT_LOAD -> UserMm -> Process/Thread -> scheduler -> U-mode/PLV3
```

Both RISC-V and LoongArch QEMU smoke tests print:

```text
M10 ELF/initramfs gate:
  newc initramfs        : verified
  ELF64 PT_LOAD         : verified
  Linux initial stack   : verified
  kernel execve path    : verified
```
