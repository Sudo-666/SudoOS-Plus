# M16-pre convergence gate

<!-- SUDOOS_M16_PRE_PATCH_V1 -->

This document is intentionally stricter than a smoke-test checklist. The goal is
not to make one QEMU run green by adding special cases; the goal is to prevent
M14/M15/M16 from being marked complete while the tree still has architectural
holes that will collapse under BusyBox, dynamic musl, SMP, or real hardware.

## Branch rule

Use `test-main` as the M14/M15/M16 integration branch unless it has been merged
back to `main` with the same docs and gates. If `main` lacks
`docs/m14-m16-progress.md`, it is not the source of truth for this phase.

## Non-negotiable gates before closing M14

M14 is not complete just because in-kernel syscall probes pass. It needs a real
static BusyBox initramfs boot in QEMU on both RISC-V64 and LoongArch64.
The minimum artifact should exercise:

- `/init` startup, `sh`, `echo`, `cat`, `ls`, `pwd`, `mkdir`, `rm`, `cp`, `mv`,
  `ln`, `sleep`, `true`, `false`, `mount`, `dmesg`, and `ps`.
- fd readiness through `ppoll`/`pselect6`, including timeout and signal-mask
  rules where musl depends on them.
- `rt_sigreturn` and the known signal limitations should stay documented until
  `siginfo`, `ucontext`, altstack, restart, and threaded selection rules exist.

## Non-negotiable gates before closing M15

M15 is not complete when ext4 magic can be read. A Linux-like flow is:

`virtio-blk -> request queue -> bounded buffer/page cache -> lwext4 blockdev -> ext4 superblock/inode/dentry/file ops -> VFS -> persistent rootfs`

The ext4 VFS gate must cover real file operations: lookup, open, read, write,
truncate or clear error return, getdents/stat, link/symlink, rename/unlink,
fsync/writeback, and remount/error behavior. Keep raw MMIO physical addresses
out of drivers; use ioremap/fixmap-style virtual mappings and a DMA HAL with
address-mask, cache-coherency, and barrier semantics.

## Non-negotiable gates before closing M16

M16 is not complete until a dynamically linked musl user program runs through the
actual dynamic loader path. Required loader surface:

- ELF accepts `ET_DYN` in addition to static `ET_EXEC`.
- `PT_INTERP`, `PT_PHDR`, and `PT_DYNAMIC` are parsed and passed to exec.
- A load-bias/layout policy maps main ET_DYN and shared objects without crossing
  user/kernel boundaries.
- Initial stack is Linux-like: `argc`, `argv[]`, NULL, `envp[]`, NULL, auxv,
  strings, `AT_RANDOM`, `AT_EXECFN`, and ABI alignment.
- Auxv includes at least `AT_PHDR`, `AT_PHENT`, `AT_PHNUM`, `AT_BASE`,
  `AT_ENTRY`, `AT_PAGESZ`, `AT_RANDOM`, and `AT_EXECFN`.
- TLS, dynamic relocations, and RELRO/mprotect behavior have explicit gates.

## Commands added by the patch

```sh
make m16-preflight
make verify-m16-pre
make busybox-initramfs BUSYBOX=/absolute/path/to/static/busybox
```

`m16-preflight` is allowed to fail while M15/M16 are incomplete. A failure is a
useful stop sign: do not rename probe plumbing into a completed milestone.
