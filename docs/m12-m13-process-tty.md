# M12/M13 process I/O, signal base, and console TTY

M12/M13 was rebuilt on top of the current M10/M11 Process/UserMm/VFS stack. The
remote `m12-m13` branch was reviewed but not merged: it was based on the older
pre-VFS file table, used `UnsafeCell` in pipe/TTY paths without the current
lockdep model, and wrote signal frames directly through raw user pointers.

## Implemented boundary

- `pipe2(2)` creates VFS-backed read/write endpoints with shared ring-buffer
  state, endpoint refcount release, EOF after the last writer closes, `EPIPE`
  when readers disappear, and `O_CLOEXEC`/`O_NONBLOCK` split across fd flags and
  file status flags. Blocking read/write paths now sleep on wait queues from
  user-trap context instead of spinning or returning fake success.
- `clone(2)` supports the first Linux-like fork subset: independent eagerly
  copied `UserMm`, copied fd table open-file descriptions, copied cwd/root and
  signal mask, child trap-frame resume with return value `0`, and parent return
  value set to the child PID. Unsafe thread-sharing flags such as `CLONE_VM`,
  `CLONE_FILES`, `CLONE_SIGHAND`, and `CLONE_THREAD` are rejected until the
  ownership model is ready for them.
- User-triggered `execve(2)` replaces the current running image through the ELF
  loader, switches the scheduler's loaded-mm anchor in place, closes `CLOEXEC`
  descriptors, resets the current user PC/SP, and destroys the old address
  space after the commit. `/init` is covered by the embedded static executable
  path used by the smoke verifier.
- `wait4(2)` blocks on the process child wait queue, reaps zombie children, and
  copies status through checked user-copy helpers.
- Process lookup, process groups, sessions, parent/zombie-child bookkeeping
  anchors, `getpid`, `getppid`, `setsid`, `setpgid`, `getpgid`, `getsid`,
  `wait4`, `kill`, `tkill`, and `tgkill` now have Linux-numbered syscall
  entries.
- Signal state is process-owned with pending and blocked masks. `rt_sigaction`
  and `rt_sigprocmask` validate signal numbers and use checked user copies.
- `/dev/console` is backed by a canonical TTY line discipline with echo,
  backspace, Ctrl-C foreground process-group delivery, and
  `TIOCGPGRP`/`TIOCSPGRP` ioctl hooks.
- Musl/BusyBox-adjacent probes for `clock_gettime`, `nanosleep`, `uname`, and
  `getrandom` are wired through the Linux generic ABI.

## Deliberate remaining edges

`clone(2)` intentionally implements fork-like process creation only. It does not
share address spaces, fd tables, signal handlers, TLS, or thread groups yet; the
kernel rejects those flags instead of faking semantics it cannot preserve.

The first fork uses eager full-page copying rather than COW. This is slower, but
it keeps the scheduler/MM/fd ownership rules simple while the syscall ABI is
being stabilized. COW can be added later on top of the existing page-fault and
TLB-shootdown infrastructure.

Signal delivery now has the first real user ABI path: process-directed pending
signals, per-thread masks, `rt_sigaction`, a one-argument user handler frame,
and `rt_sigreturn` are covered by the RISC-V and LoongArch smoke probes. This is
still not full POSIX/Linux signal machinery: `siginfo`, `ucontext`, altstack,
syscall restart, and threaded signal selection remain future work.

The TTY is canonical-only for now. Raw mode, termios, poll/select readiness, and
complete shell job-control behavior are still future work.

## Robustness fixes made during M12/M13

- Added `FileOperations::release()` so file-like endpoints can observe final
  close without relying on fd-table internals.
- Increased guarded scheduler kernel stacks from 16 KiB to 32 KiB after
  LoongArch exposed a real trap-frame write into the lower guard page while
  entering the enlarged user ABI verifier. Guard pages remain active.
- Kept all user memory traffic on checked `copy_to_user`/`copy_from_user`
  helpers; no signal or TTY path writes through unchecked user pointers.
- Fixed fd-table lifetime ordering so closing a pipe/TTY file, replacing an fd,
  or applying `CLOEXEC` removes the descriptor under the process fd-table lock
  but drops the final file reference after that lock is released. This avoids
  lockdep violations when `release()` wakes wait queues.

## Verification

The debug boot now reports:

```text
M12 pipe gate:
  ring buffer          : verified
  EOF after writer drop: verified
  pipe2 status flags   : verified
M12 signal gate:
  signal set/mask      : verified
  pending delivery core: verified
M13 TTY gate:
  canonical input      : verified
  console output       : verified
M12/M13 user ABI gate:
  clone/wait4          : verified
  execve current image : verified
  pipe2/read/write      : verified
  pid/session syscalls  : verified
  clock/uname/getrandom : verified
```

Both `make smoke-riscv64` and `make smoke-loongarch64` pass with this gate.
