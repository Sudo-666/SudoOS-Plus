# M14-M16 progress

This note records the current M14-M16 boundary. It is intentionally
conservative: the tree now has real static BusyBox artifacts, external
initramfs plumbing on the RISC-V QEMU path, a native read-only ext4 snapshot
handoff into VFS, and M16-A ELF/auxv/static-PIE relocation support. Full
dynamic userland remains a later boundary until PT_INTERP, shared-object mmap,
TLS, and RELRO all run through real QEMU smoke.

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
- Vendor static BusyBox artifacts exist for RISC-V and LoongArch under
  `vendor/userland/*/busybox-static`, and the deterministic rootless newc
  builder emits `/init -> /bin/busybox`, applet symlinks, and the basic runtime
  directories.
- RISC-V QEMU `-initrd build/initramfs/busybox-riscv64.cpio` is parsed through
  `/chosen/linux,initrd-start/end`, reserved from the physical page allocator,
  and unpacked into the tmpfs rootfs before user verification.
- Initramfs parsing now understands regular files, directories, and symlinks;
  exec-from-initramfs follows symlinks with a bounded resolver.
- Ext4 M15-A is no longer a magic-only probe: the native read-only parser walks
  superblock/group/inode/extent/directory state and installs a snapshot into
  VFS with EROFS guards.
- The block layer exposes a first request-queue shape, bounded buffer/page
  cache, dirty flush, byte-range I/O, and explicit virtio DMA ordering markers.
- `pselect6` now handles fd_set readiness through the VFS poll hook for the
  current nonblocking surface. The signal ABI has explicit siginfo/ucontext and
  altstack boundary structs, while unsupported full POSIX behavior remains
  fail-closed instead of being silently claimed.
- Exec now copies real `argv[]`/`envp[]` from userspace, builds a Linux-shaped
  initial stack for all arguments and environment strings, and applies
  no-interpreter static PIE `R_RELATIVE` relocations. `PT_INTERP` binaries still
  fail closed instead of entering an incomplete dynamic-loader path.
- The real vendor RISC-V BusyBox static PIE is now executed from the unpacked
  external initramfs during QEMU smoke. The gate runs `/bin/busybox true` and
  waits for exit status 0 from a normal user task rather than a synthetic
  verifier-only image.

## Validation

- `make check`
- `make smoke-all`
- `make smoke-smp-all`
- `make m14-vendor-userland-audit-strict`
- `make busybox-initramfs-vendor-all`
- `make m15a-ext4-ro-audit`
- `make m16a-audit`
- `make m16-preflight`
- `QEMU_ARGS='-initrd build/initramfs/busybox-riscv64.cpio' make smoke-riscv64`
- `QEMU_ARGS='-drive file=/private/tmp/sudoos-virtio.raw,if=none,format=raw,id=hd0 -device virtio-blk-device,drive=hd0' make smoke-riscv64`

Both RISC-V and LoongArch pass single-core and SMP QEMU smoke after these
changes. The extra RISC-V run verifies a real virtio-blk device is discovered
and registered; it intentionally does not perform destructive writes to the
attached disk.

## Still not complete

Not complete means the feature is not yet Linux-compatible enough to close the
stage, even if the current plumbing and QEMU smoke gates pass.

### Static BusyBox as PID 1

RISC-V can now receive and unpack the external BusyBox initramfs, execute the
vendor static PIE BusyBox from `/bin/busybox`, and verify the `true` applet
through normal process exit. The remaining work is to publish `/init` as the
long-lived boot user process and broaden applet smoke to `sh`, `ls`, `ln`,
`sleep`, and `ps`. Keep the old verifier on its private `/.m12` exec probe;
`/init` is reserved for real userland.

LoongArch direct boot currently accepts `-initrd` without crashing, but QEMU's
FDT in this path does not expose `linux,initrd-start/end`, so the kernel has no
trustworthy initrd descriptor to import yet.

### Persistent ext4

M15-A provides a native read-only ext4 snapshot into VFS. The remaining
persistent-filesystem work is journal/replay policy, htree directories,
xattrs/ACLs, writeback, crash-consistency boundaries, and a persistent rootfs
mount path. A future lwext4 adapter may still be useful for parity tests, but
it must enter through the same VFS inode/dentry/file operation shape. Do not
bypass VFS with ad-hoc ext4 reads.

### Dynamic musl

M16-A parses `ET_DYN`, `PT_INTERP`, `PT_PHDR`, `PT_DYNAMIC`, load bias, builds
Linux-like auxv entries, and supports no-interpreter static PIE `R_RELATIVE`
relocations. M16-B still needs PT_INTERP loading, loader/shared-object mmap
layout, symbol relocations, TLS setup, and RELRO `mprotect` handling before
dynamic musl can be considered complete.
