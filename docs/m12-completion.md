# M12 completion: file descriptor table, pipe, signal, process control

M12 lands four new subsystems on top of M9's Process/Thread ownership:
file descriptor table, pipe, signal delivery, and process control
(fork/exit/wait/session). All syscall handlers use the existing `Arc<Process>`
+ `IrqSpinLock` model without introducing new lock ranks.

## File descriptor table (`file_table.rs`)

```text
FileTable (per-Process, IrqSpinLock<FileTable>)
├── Vec<Option<File>>          // 64 slots, sparse allocation
│   └── File
│       ├── Arc<dyn FileOperations>   // pipe, TTY, future regular files
│       ├── OpenFlags                 // RDONLY/WRONLY/NONBLOCK/CLOEXEC
│       └── refcount: AtomicUsize     // embedded, no extra allocation
```

`File` carries an embedded atomic refcount. `FileTable::clone()` copies every
slot, bumping each File's refcount, so a forked child shares open file
descriptions exactly as Linux does.

`alloc_fd()` scans for the first free slot, then appends. `take_fd()` removes a
File from the table without dropping it, allowing the caller to perform the
final drop outside any Process-rank lock — this is the key lock-ordering
contract for pipe teardown.

## Pipe (`pipe.rs`)

```text
Pipe (Arc-shared between reader and writer)
├── buffer: UnsafeCell<[u8; 4096]>     // Linux default PIPE_BUF_SIZE
├── head / len: UnsafeCell<usize>      // ring-buffer cursors
├── reader_open / writer_open: AtomicBool
├── read_wait: WaitQueue               // readers block when empty
└── write_wait: WaitQueue              // writers block when full
```

`PipeReader` / `PipeWriter` implement `FileOperations`. The pipe buffer is
protected by UnsafeCell rather than a mutex — the kernel's single-threaded
per-CPU execution model guarantees exclusive access during the syscall.

`create_pipe(flags)` returns `(reader_file, writer_file)`. The caller installs
both into the fd table via `sys_pipe2`.

**EOF and SIGPIPE:** `do_read` returns 0 (EOF) when the buffer is empty and the
writer is closed. `do_write` returns `BrokenPipe` when the reader is closed.
The signal path for SIGPIPE is wired in `signal.rs` but not yet triggered from
the pipe write path (deferred until VFS-aware writev integration).

## Signal subsystem (`signal.rs`)

```text
SignalState (per-Process, IrqSpinLock<SignalState>)
├── pending: SigSet(u64)          // Linux-compatible 64-bit sigset_t
├── blocked: SigSet(u64)          // current signal mask
└── actions: [SigAction; 33]      // NSIG = 33 (SIGHUP..SIGSYS)
    └── SigAction (#[repr(C)], 32 bytes)
        ├── handler: usize        // SIG_DFL=0, SIG_IGN=1, or user fn
        ├── flags: u64            // SA_NODEFER, SA_RESETHAND
        ├── restorer: usize       // __vdso_rt_sigreturn or libc
        └── mask: SigSet          // blocked during handler execution
```

**Signal delivery** (`do_signal`): on return to user mode, the trap handler
scans pending & ~blocked. SIG_DFL terminates the process (except SIGCHLD,
SIGURG, SIGWINCH, SIGCONT which are ignored by default). SIG_IGN clears the
pending bit. A user handler triggers `setup_rt_frame` which saves the current
`TrapFrame` on the user stack, points `sepc`/`era` at the handler, `ra` at the
restorer (which calls `rt_sigreturn`), and `a0` at the signal number.

**rt_sigreturn** restores the original TrapFrame from the user stack and clears
the signal mask.

**ABI compatibility:** `SigAction` layout matches Linux's `struct sigaction`
(32 bytes: 8 handler + 8 flags + 8 restorer + 8 mask). Serialization helpers
(`copy_sigaction_from_user`, etc.) accept an explicit `&UserMm` to avoid
calling `current_user_thread()` → `SCHEDULER.lock()` inside a Process-locked
closure — this resolves the Process/#5 → Scheduler/#1 lockdep violation.

## Process control

```text
Process extension (M12 fields on the M9 struct)
├── parent: IrqSpinLock<Option<ProcessId>>
├── children: IrqSpinLock<Vec<ProcessId>>
├── proc_state: AtomicU8           // Running(0) / Zombie(1)
├── proc_exit_code: AtomicU32
├── pgrp: AtomicI32                // process group ID
├── session: AtomicI32             // session ID (0 = no session)
├── comm: IrqSpinLock<[u8; 16]>   // process name
└── program_break: AtomicUsize
```

A global `PROCESS_REGISTRY` (IrqSpinLock<BTreeMap<ProcessId, Weak<Process>>>)
provides PID→Process lookup for signal delivery. A per-process zombie queue
`ZOMBIE_QUEUE` collects exited children for `wait4`.

**fork_process:** copies VMA metadata (not pages — the child demand-faults),
clones the file table (refcount bump), and forks the signal state (pending
cleared, blocked/actions inherited). Full COW is deferred.

**exit_process:** takes all files out of the table under lock, drops them
outside, marks the process zombie, pushes it onto the zombie queue, and sends
SIGCHLD to the parent. The take-outside-drop pattern avoids Process/#4 →
WaitQueue/#1 lock inversion.

**wait_child:** scans the zombie queue for a child of the caller, reaps it,
and returns its exit code.

**Session / process group:** `setsid`, `setpgid`, `getpgid`, `getpgrp`,
`getsid` follow Linux semantics. `setsid` is rejected when the caller is
already a process group leader.

## ELF loader (`elf.rs`)

```text
load_elf(data: &[u8], user_mm: &UserMm) -> ElfLoadInfo
├── parse ELF64 header (magic, 64-bit, LE, ET_EXEC, arch match)
├── parse program headers
├── map PT_LOAD segments via UserMm::map_fixed_area()
└── ElfLoadInfo { entry, phdr, phnum, brk_end }
```

`setup_user_stack()` lays out argc, argv[], envp[], and auxv[] (AT_PAGESZ,
AT_PHDR, AT_PHNUM, AT_ENTRY, AT_RANDOM, AT_PLATFORM) on the initial user
stack, Linux `create_elf_tables` compatible. The loader is gated behind VFS or
initramfs availability — `sys_execve` currently returns `-ENOSYS` until a
file-backed read path exists.

## Syscall ABI expansion

M12 extends the Linux asm-generic 64-bit table:

| Category | Syscalls | Numbers |
|----------|----------|---------|
| File I/O | read, write, close, dup | 63, 64, 57, 23 |
| Pipe | pipe2 | 59 |
| Process | clone, execve, wait4 | 220, 221, 260 |
| Signal | rt_sigaction, rt_sigprocmask, rt_sigreturn, kill, tkill, tgkill | 134, 135, 139, 129, 130, 131 |
| Identity | getpid, getppid | 172, 173 |
| Time | nanosleep, gettimeofday, clock_gettime, times | 101, 169, 113, 153 |
| System | uname, getrandom | 160, 278 |

## Lock-ordering contract

SudoOS-Plus enforces `Scheduler(20) < WaitQueue(30) < Process(35) < Vm(40) <
Console(80)`. M12 code obeys this by:

1. **Pipe I/O**: `sys_read`/`sys_write` look up the `File` under
   `with_files_mut` (Process/#4), release the lock, then call
   `file.read()`/`file.write()`. `sys_close` uses `take_fd()` to remove the
   File under lock and drop it outside.

2. **Signal ABI**: `copy_sigaction_from_user` and friends accept `&UserMm` as
   a parameter. The caller pre-fetches the mm reference before entering any
   Process-locked closure, so `current_user_thread()` → `SCHEDULER.lock()`
   never executes inside Process/#5.

3. **Signal delivery**: `do_signal` is called from the trap return path, which
   holds no Process locks. `send_signal` calls `request_reschedule_local()`
   only after releasing the process registry and signal locks.

4. **Process teardown**: `exit_process` takes the fd table contents out under
   lock, then drops Files outside. File drops may trigger
   `Pipe::close()` → `wake_all()` → WaitQueue/#1, but no Process lock is held.

## Closure gate

M12 is verified when both architectures pass build, Clippy, SMP=1/4 QEMU
smoke, all M9 regression audits, and the following user-mode test sessions:

| Test | Syscalls exercised |
|------|--------------------|
| pipe2/write/read/close | pipe2(59), write(64), read(63), close(57)×2 |
| sigaction/sigprocmask/kill | rt_sigaction(134), rt_sigprocmask(135)×2, getpid(172), kill(129) |
| getpid/getppid | getpid(172), getppid(173) |
