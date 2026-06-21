# M9 completion: Process/Thread, scheduler MM, Linux 64-bit syscall ABI

M9 closes the temporary M8 verifier boundary without replacing the verified
ASID, `active_cpus`, demand-fault, and TLB-before-free implementation.

## Ownership

```text
Scheduler Task
├── guarded KernelStack
├── Arc<Thread>
│   ├── Arc<Process>
│   │   ├── Arc<UserMm>
│   │   ├── FileTable
│   │   ├── SignalState
│   │   ├── Credentials
│   │   └── FsContext
│   ├── user PC / user stack
│   ├── TLS / blocked signal mask
│   └── exit state
└── join Completion
```

The process thread group stores IDs rather than `Arc<Thread>`, avoiding a
Process/Thread strong-reference cycle. The scheduler reaper destroys the retired
kernel stack and releases its `Arc<Thread>` before publishing join completion.
The verifier/process owner can therefore drop the final Thread reference and
tear down the Process/UserMm only after no retired scheduler object can use it.

## `switch_mm_irqs_off()` contract

Each CPU owns `loaded_mm: Option<Arc<UserMm>>`. Under the Scheduler lock with
local interrupts disabled, a context switch performs:

1. verify the outgoing user Task matches the CPU's `loaded_mm`;
2. retain the root for a same-MM switch;
3. otherwise restore the kernel root and clear the old MM's active CPU bit;
4. install/synchronize the incoming root and ASID;
5. publish the CPU in the new MM's active mask;
6. only then switch kernel stacks/contexts.

A user Task is retired only after it has switched to a kernel/other MM. Final
UserMm destruction still requires an empty active CPU mask and completes every
TLB request before freeing backing or page-table pages.

## Trap and ABI contract

RISC-V returns to U-mode with SPIE set; LoongArch returns to PLV3 with PIE set.
Timer and IPI interrupts can therefore preempt a user Task on its own guarded
kernel stack. Trap return reconstructs the scratch-CSR kernel-stack pointer, so
resume is safe on the CPU selected by the scheduler.

The centralized asm-generic ABI includes `write`, `exit`, `exit_group`,
`sched_yield`, `brk`, `munmap`, `mmap`, and `mprotect`. The deterministic M9 probe pins a peer and a user Task to one CPU. The
peer first publishes a Completion-based ready handshake, then each of eight
Linux `sched_yield` syscalls verifies that the user Thread's schedule counter
increased before the syscall returns. This proves a real
user→kernel-peer→user switch without depending on whether the initial remote
activation is included in an aggregate schedule count.

## Closure gate

M9 is complete only when both architectures pass build, Clippy, SMP=1/4 QEMU
smoke, M8 regression audits, and `scripts/m9b-audit.py`. M10 may now add ELF64,
`execve`, the initial user stack, and initramfs without redesigning task/MM
ownership.

### Non-unwinding bootstrap ownership

The fresh user-task bootstrap switches out through `exit_current()` and never
unwinds its Rust stack frame. Any temporary `Arc<Thread>` obtained from the
scheduler must therefore be dropped explicitly before that switch. The task
reaper then releases the scheduler-owned `Arc<Thread>` before publishing join
completion, leaving the verifier/session owner as the only remaining thread
reference.

## RISC-V privilege-entry identity contract

RISC-V `tp` is kernel-owned while executing supervisor code and names the
logical CPU. User mode owns its own `tp` value for TLS. Therefore a user trap
must restore kernel `tp` before any Rust code calls `current_cpu_id()` or
accesses per-CPU scheduler state.

While user code runs, `sscratch` points at a 24-byte task-stack anchor holding
the saved kernel SP and kernel `tp`. Trap entry consumes the anchor, preserves
user `tp` in `TrapFrame`, restores kernel `tp`, and clears `sscratch`. Before
`sret`, the anchor is rebuilt with the destination CPU's current `tp`; this
keeps migration correct. This follows Linux RISC-V's core rule that privilege
entry swaps user thread state for kernel current/per-CPU state before C code.

