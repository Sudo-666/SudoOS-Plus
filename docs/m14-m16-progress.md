# M14-M16 progress

This note records the current state after the M14 foundation pass and the M15
block-device/VFS mount-table pass. It is intentionally conservative: M15's
block-device plumbing is in place, but M15 is not a complete persistent ext4
filesystem until lwext4-backed inode/file operations run through VFS. M16 is
not complete until dynamically linked userland runs.

## Completed in this pass

- Tmpfs now supports Linux-like symbolic links and hard links:
  `symlinkat`, `linkat`, `readlinkat`, final-component `O_NOFOLLOW`, `lstat`
  through `newfstatat(..., AT_SYMLINK_NOFOLLOW)`, and a bounded 40-hop symlink
  resolver.
- File readiness has a first VFS hook and `ppoll` support. Pipe readiness now
  reports read, write, hangup, error, and invalid-fd conditions instead of
  treating all descriptors as permanently ready.
- The console TTY has minimal termios and window-size ioctls:
  `TCGETS`, `TCSETS*`, `TIOCGWINSZ`, `TIOCSWINSZ`, plus the existing foreground
  process-group ioctls.
- BusyBox/musl-adjacent syscalls are wired where the current kernel can provide
  real semantics: `gettid`, uid/gid queries, `set_tid_address`,
  `set_robust_list`, `faccessat`, `prlimit64`, `sysinfo`, `nanosleep`,
  `symlinkat`, `linkat`, `ppoll`, and `fcntl(F_DUPFD_CLOEXEC)`.
- `mount(2)`/`umount2(2)` now update a bounded in-kernel mount table for
  `tmpfs`, `devtmpfs`, `proc`, and ext4 probe mounts. Duplicate mountpoints
  fail with `EBUSY`; `/` cannot be unmounted.
- A first Linux-like block layer exists: `BlockDevice`, a bounded buffer cache,
  a page-cache wrapper, dirty writeback/`fsync`, byte-range I/O, block-size and
  range validation, and a memory-backed verifier.
- `vendor/virtio-drivers` is now a real path dependency. The kernel collects
  FDT `virtio,mmio` regions before heap initialization, maps each region with
  `ioremap`, constructs the upstream MMIO transport, and ignores empty QEMU
  slots without panicking.
- The virtio HAL allocates zeroed DMA32 pages through the kernel page allocator,
  tracks DMA allocations until `dma_dealloc`, and translates direct-map/kernel
  image buffers for QEMU's coherent virtio-mmio transport.
- `VirtIOBlk` from the vendor driver is wrapped as a kernel `BlockDevice`.
  A RISC-V QEMU run with an attached raw virtio-blk disk registers `/dev/vda`
  successfully, and devfs installs registered block devices before userland.
- The ext4 path has a real superblock magic gate through block-device reads:
  `mount(..., "ext4", ...)` requires an existing block device and the `0xef53`
  magic at the Linux ext4 superblock offset. This is a probe gate, not a full
  lwext4-backed filesystem yet.
- Basic user signal delivery now builds a checked user signal frame, enters a
  one-argument handler, blocks the delivered signal while the handler runs, and
  restores the saved trap frame and old mask through `rt_sigreturn`. The
  LoongArch embedded user image is page-aligned so page-relative address
  materialization remains valid after copying the verifier image to `USER_CODE`.

## Validation

- `make check`
- `make smoke-all`
- `make smoke-smp-all`
- `QEMU_ARGS='-drive file=/private/tmp/sudoos-virtio.raw,if=none,format=raw,id=hd0 -device virtio-blk-device,drive=hd0' make smoke-riscv64`

Both RISC-V and LoongArch pass single-core and SMP QEMU smoke after these
changes. The extra RISC-V run verifies a real virtio-blk device is discovered
and registered; it intentionally does not perform destructive writes to the
attached disk.

## Not complete yet

### M14 static BusyBox

The kernel is much closer to static BusyBox, but it still needs a real BusyBox
initramfs test artifact and a user-mode smoke that runs applets such as `sh`,
`ls`, `ln`, `sleep`, and `ps`. Basic handler delivery and `rt_sigreturn` are
implemented, but `siginfo`, `ucontext`, altstack, syscall restart, and the full
threaded signal-selection rules remain incomplete.

### M15 virtio-blk and ext4

Current state:

- Done: FDT discovery, `ioremap`-backed MMIO transport probe, vendor
  `virtio-drivers` HAL/DMA, `virtio-blk` initialization, kernel `BlockDevice`
  wrapper, bounded buffer/page cache, byte-range I/O, `fsync` writeback, mount
  table, block devfs nodes, and ext4 superblock validation.
- Not done: request merging/scheduling, interrupt-driven block I/O, full ext4
  integration through `vendor/lwext4`, ext4 inode/dentry/file operations, and a
  persistent root filesystem.

The remaining Linux-like path is:

```text
virtio-blk block device
  -> lwext4 blockdev adapter
  -> ext4 inode/dentry/file operations
  -> persistent root filesystem
```

Do not bypass `ioremap` by handing raw MMIO physical addresses to the driver.
On RISC-V the final direct MMIO helper is intentionally narrow and only covers
the early UART fixmap path.

### M16 dynamic musl

The current ELF loader accepts static `ET_EXEC` with `PT_LOAD`. M16 still needs
`ET_DYN`, `PT_INTERP`, loader handoff auxv entries, shared-object mmap layout,
TLS setup, dynamic relocations, and RELRO `mprotect` handling before dynamic
musl can be considered complete.
