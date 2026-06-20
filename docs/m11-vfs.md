# M11 VFS completion notes

M11 starts from the remote `origin/vfs` branch, but does not merge it directly:
that branch was based before the current M8-M10 Process/UserMm/exec work and
would remove critical ownership, ASID, and TLB-shootdown code.

## Reused direction

- Linux-like names and layering: open files, per-process fd table, tmpfs root,
  devfs nodes, Linux errno/open/seek/stat constants.
- The M11 smoke gate covers tmpfs file operations, directory enumeration,
  rename/unlink, devfs, cwd path resolution, and per-process fd table
  installation.

## Rewritten for robustness

- The VFS crate uses `Arc<File>` instead of hand-written `AtomicU32 + *const T`
  reference counts.
- `dup` shares one open file description and therefore one locked file offset,
  matching Linux `struct file` semantics.
- File operations are `Send + Sync` and use interior locking instead of exposing
  unsound `&mut File` access through duplicated descriptors.
- Kernel tmpfs/devfs nodes use explicit checked allocation for directory entry
  names and vectors.
- Syscalls now resolve open file descriptions through the process fd table;
  `write(2)` reaches `/dev/console` through devfs instead of a hard-coded fd
  special case.
- Directory-changing tmpfs operations are serialized by a VFS tree lock before
  taking node locks, keeping `mkdir`, `unlink`, and `rename` structurally
  consistent on SMP.
- `rename` validates replacement type and non-empty directory cases before
  mutating the tree, and pre-reserves the target slot for cross-directory moves
  so allocation failure cannot silently drop the source entry.

## Completed M11 surface

Implemented now:

- `myos-vfs` crate: `Errno`, `OpenFlags`, `FileMode`, `SeekWhence`, `Stat`,
  Linux `dirent64` emission, `IoBuffer`, `MutableIoBuffer`, `FileOperations`,
  `File`, and `FileTable<MAX_FDS>`.
- Kernel tmpfs/devfs root with `/dev/null`, `/dev/zero`, and `/dev/console`.
- Process-owned fd table with stdin/stdout/stderr installed during initial exec.
- Syscalls wired through VFS: `openat`, `close`, `read`, `write`, `lseek`,
  `fstat`, `newfstatat`, `getdents64`, `mkdirat`, `unlinkat`, `renameat`,
  `readlinkat` (returns `EINVAL` for non-symlinks), `chdir`, `getcwd`, `dup`,
  `dup3`, `fcntl`, `ioctl`, `fsync`, and `ftruncate`.
- Process cwd/root state and relative path normalization for `AT_FDCWD`.
- Dual-architecture user-mode probe for `openat`, `write`, `lseek`, `read`,
  and `close` through the real syscall path.

## Remaining outside M11

- Mount table, dentry/inode cache separation, and real superblock operations.
- ext4 via lwext4, virtio-blk, and persistent writeback/fsync semantics.
- Symlink/hardlink creation and following, executable permission enforcement,
  timestamps, ownership checks, and file locks.
