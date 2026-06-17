# M13 completion: console TTY, session/process group, ioctl

M13 lands the console TTY line discipline, session and process group management,
and the `ioctl` syscall for terminal job control. Together with M12's pipe and
signal infrastructure, this provides the kernel side of a minimal Unix shell.

## TTY subsystem (`tty.rs`)

```text
Tty (global SYSTEM_TTY, IrqSpinLock<Option<Tty>>)
├── read_buf: UnsafeCell<CanonBuffer>  // N_TTY_BUF_SIZE = 4096
│   └── CanonBuffer: [u8; 4096] + head/tail cursors
├── line_state: UnsafeCell<LineState>  // Idle / Complete
├── lflag: UnsafeCell<TermiosLflag>    // ICANON, ECHO, ISIG, etc.
├── foreground_pgrp: AtomicI32         // foreground process group
├── session: AtomicI32                // owning session ID
├── read_wait: WaitQueue              // readers block until line ready
└── echo state (UnsafeCell)
```

`ConsoleDriver` is a `Send + Sync` trait abstracting the hardware console
output. `set_console_driver()` registers an architecture-specific driver (wraps
`arch::early_console::write_byte`). The global `SYSTEM_TTY` and `CONSOLE_DRIVER`
are protected by `IrqSpinLock` at `LockRank::Console(80)`.

### Line discipline (N_TTY)

**Canonical mode** (`ICANON`):
- Characters accumulate in the read buffer until `\r` or `\n` commits the line.
- `^H` / DEL erases the previous character.
- `^D` on an empty line sends EOF (line committed with zero length).
- `^U` erases the entire line.
- `ECHO` writes each character back to the console; `ECHOE` echoes erase
  sequences (`\x08 \x08`).

**Raw mode** (canonical disabled):
- Each character is immediately available to readers.
- `read_wait.wake_one()` fires on every character.

**Signal generation** (`ISIG`):
| Key | Signal | Echo |
|-----|--------|------|
| `^C` (0x03) | SIGINT | `^C\r\n` |
| `^\` (0x1c) | SIGQUIT | `^\r\n` |
| `^Z` (0x1a) | SIGTSTP | `^Z\r\n` |

`signal_foreground()` sends the signal to `foreground_pgrp`. The TTY's
`foreground_pgrp` is set via `ioctl(TIOCSPGRP)` by the shell.

### User-facing file operations

`TtyReader` and `TtyWriter` implement `FileOperations`. `create_console_reader()`
and `create_console_writer()` produce File objects suitable for fd 0 (stdin) and
fd 1 (stdout). In canonical mode, `do_read` returns a completed line. In raw
mode, `do_read` returns whatever is buffered. `do_write` sends bytes to the
registered `ConsoleDriver`, appending `\r` before `\n`.

### Kernel input path

`tty::input_char(byte)` feeds a character from the hardware console into the
TTY's line discipline. This function is designed to be called from an interrupt
or polling driver. The user-facing `TtyReader::read` and `TtyWriter::write`
are called from the syscall path.

## Session and process group

M13 extends the M12 Process struct with session and process group management.
All state lives behind atomics or existing IrqSpinLocks — no new lock ranks.

```text
Process (M13 fields)
├── pgrp: AtomicI32            // process group ID
└── session: AtomicI32         // session ID (0 = no session)
```

| Syscall | Number | Behavior |
|---------|--------|----------|
| `setsid()` | 132 | Creates a new session. Rejected if caller is already a process group leader (pgrp == pid). Returns the new session ID (== pid). |
| `setpgid(pid, pgid)` | 133 | Sets `target_pid`'s pgrp. `pgid=0` means use `target_pid`. Caller must be in the same session. |
| `getpgid(pid)` | 155 | Returns the target's pgrp. `pid=0` means self. |
| `getpgrp()` | — | Alias for `getpgid(0)`. Same syscall number. |
| `getsid(pid)` | 156 | Returns the target's session ID. `pid=0` means self. |

Session validation is simplified: the caller must share a non-zero session
with the target. A full Linux implementation would check `CAP_SYS_ADMIN` or
the UID match.

## ioctl (`sys_ioctl`)

```text
ioctl(fd, request, arg) -> isize
├── TIOCGPGRP (0x540f)  → read foreground process group into *arg
└── TIOCSPGRP (0x5410)  → set foreground process group from *arg
```

`TIOCGPGRP` reads the TTY's `foreground_pgrp` and copies it to user space.
`TIOCSPGRP` copies a pgrp from user space and sets it on the TTY. Both
validate that `fd == 0` (stdin). The TTY lock scope is minimized — the
user-space copy happens outside the lock to avoid `Console/#3 →
Scheduler/#1` lock inversion.

The pid for the no-TTY fallback in `TIOCGPGRP` is pre-fetched before the
TTY lock is acquired.

## Lock-ordering contract

Console-rank locks (80) sit above all other ranks. M13 code never holds a
Console lock while acquiring Scheduler (20), Process (35), or Vm (40):

1. **ioctl**: `current_pid()` and `current_user_mm()` are called before
   `system_tty().lock()`. The TTY lock is scoped to a single atomic read;
   `copy_to_user` happens outside.

2. **tty::input_char**: acquires `SYSTEM_TTY` (Console/#3) to push a character
   through the line discipline. `signal_foreground` calls `send_signal`, which
   acquires Process/#2 → Process/#5, then releases both before calling
   `request_reschedule_local()`. WaitQueue wake operations (`read_wait.wake_all`)
   acquire WaitQueue/#1 which is below Process and Console.

## Syscall ABI

M13 adds one syscall beyond M12:

| Syscall | Number | Purpose |
|---------|--------|---------|
| `ioctl` | 29 | TIOCGPGRP / TIOCSPGRP for terminal job control |

Session/process group syscalls (`setsid`, `setpgid`, `getpgid`, `getsid`) use
numbers already listed in M12's table.

## Closure gate

M13 is verified when both architectures pass build, Clippy, SMP=1/4 QEMU
smoke, all M9/M12 regression audits, and the following user-mode test session:

| Test | Syscalls exercised |
|------|--------------------|
| session/ioctl | getpid(172), getpgid(155), getsid(156), ioctl(29) |

Future M13+ work: interrupt-driven console input (wiring a hardware IRQ to
`tty::input_char`), full termios `tcgetattr`/`tcsetattr`, job control
(`SIGTSTP`/`SIGCONT` handshake), and `/dev/tty` device node.
