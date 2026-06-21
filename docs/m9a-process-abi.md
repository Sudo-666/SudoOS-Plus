# M9-A: Process/Thread ownership and Linux syscall ABI

Exact baseline: `db6fa3b30f67bd70948d12a204a34b6d5bab640e`.

M9-A preserves the complete M8 `UserMm`, ASID, `active_cpus`, page-fault, and
per-mm TLB implementation. It changes ownership and centralizes syscall ABI
facts before scheduler-driven MM switching is introduced.

## Ownership graph

```text
UserImage (temporary synchronous verifier harness)
├── Arc<Process>
│   ├── Arc<UserMm>
│   ├── FileTable anchor
│   ├── SignalState
│   ├── Credentials
│   ├── FsContext (root/cwd anchors)
│   └── ThreadGroup: Vec<ThreadId>
└── Arc<Thread>
    ├── Arc<Process>
    ├── user PC and user-stack range
    ├── architecture TrapFrame slot
    ├── TLS and blocked-signal mask
    ├── scheduler-task binding slot
    └── READY/RUNNING/EXITING/EXITED lifecycle
```

The process stores thread IDs rather than `Arc<Thread>`, so the graph cannot
form a strong-reference cycle. The initial thread follows Linux's leader rule:
`TID == PID`.

Teardown is explicit:

```text
Thread RUNNING -> EXITING -> EXITED
    -> drop final Thread owner
    -> detach TID from ThreadGroup
    -> Arc::try_unwrap(Process)
    -> Arc::try_unwrap(UserMm)
    -> UserMm::destroy()
```

A `Process` lock rank sits between `WaitQueue` and `Vm`. Process locks are
released before MM teardown enters VM, page-table, allocator, or TLB paths.

## ABI contract

`kernel/src/syscall.rs` is the sole owner of:

- Linux asm-generic syscall numbers used by the current kernel;
- positive errno constants and Linux negative-error encoding;
- RISC-V syscall number, argument, and result registers;
- LoongArch syscall number, argument, and result registers;
- four-byte syscall-PC advancement.

The M8 dispatcher remains intact but delegates register decode/result placement
to that ABI module.

## M9-B closure

M9-A did not claim scheduler-integrated user threads. M9-B now closes the
following originally deferred requirements:

1. bind `Thread` to one scheduler task and guarded kernel stack;
2. separate process/MM ownership from per-CPU loaded-MM state;
3. add reviewed `switch_mm_irqs_off()` entry/exit ordering;
4. support user-thread migration and shared-mm active CPU publication;
5. replace the verifier-only global `ACTIVE_MM` lookup;
6. verify scheduler-owned MM switching and teardown under SMP smoke.

The resulting invariants and release gate are documented in
[`m9-completion.md`](m9-completion.md).

## Toolchain compatibility gate

The M9-A verified patch also removes three warnings promoted to errors by the
current project nightly and `clippy -D warnings`: the blank line after the
GROWSDOWN doc comment, a duplicated RISC-V `#[inline]`, and a direct function
item-to-integer cast. These are narrow source hygiene fixes; they do not change
M8 MM or architecture behavior.
