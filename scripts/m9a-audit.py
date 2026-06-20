#!/usr/bin/env python3
"""Audit the M9-A Process/Thread ownership and syscall ABI boundary."""

from __future__ import annotations

from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]


def read(relative: str) -> str:
    path = ROOT / relative
    if not path.is_file():
        raise SystemExit(f"M9-A audit: missing {relative}")
    return path.read_text(encoding="utf-8")


def check(name: str, condition: bool, failures: list[str]) -> None:
    if condition:
        print(f"  PASS: {name}")
    else:
        print(f"  FAIL: {name}")
        failures.append(name)


def main() -> int:
    main_rs = read("kernel/src/main.rs")
    lockdep = read("kernel/src/lockdep.rs")
    process = read("kernel/src/process.rs")
    syscall = read("kernel/src/syscall.rs")
    user = read("kernel/src/user.rs")
    user_mm = read("kernel/src/user_mm.rs")
    riscv_paging = read("arch/riscv64/src/memory/paging/mod.rs")
    loongarch_paging = read("arch/loongarch64/src/memory/paging/hardware.rs")
    loongarch_smp = read("arch/loongarch64/src/smp.rs")
    tlb = read("kernel/src/tlb.rs")
    user_space = read("mm/src/user_space.rs")
    design = read("docs/m9a-process-abi.md")

    failures: list[str] = []

    check("process module is wired", "mod process;" in main_rs, failures)
    check("syscall module is wired", "mod syscall;" in main_rs, failures)
    check("Process lock rank exists", "Process = 35" in lockdep, failures)
    check(
        "WaitQueue < Process < Vm is compile-time asserted",
        all(
            marker in lockdep
            for marker in (
                "LockRank::WaitQueue as usize) < (LockRank::Process as usize",
                "LockRank::Process as usize) < (LockRank::Vm as usize",
            )
        ),
        failures,
    )

    check(
        "Process owns Arc<UserMm>",
        "mm: Arc<UserMm>" in process or "mm: IrqSpinLock<Arc<UserMm>>" in process,
        failures,
    )
    check("Thread owns Arc<Process>", "process: Arc<Process>" in process, failures)
    check(
        "thread group stores IDs rather than Arc<Thread>",
        "members: Vec<ThreadId>" in process
        and "Vec<Arc<Thread>>" not in process,
        failures,
    )
    check(
        "leader follows TID == PID",
        "let id = ThreadId(self.id.get());" in process,
        failures,
    )
    check(
        "Process teardown unwraps and explicitly destroys UserMm",
        "Arc::try_unwrap(mm)" in process and "mm.destroy()?;" in process,
        failures,
    )
    check(
        "thread lifecycle publishes exit status before EXITED",
        re.search(
            r"exit_status\.store\(status, Ordering::Relaxed\);\s*"
            r"self\.lifecycle\.store\(THREAD_EXITED, Ordering::Release\);",
            process,
        )
        is not None,
        failures,
    )
    check(
        "Thread Drop detaches from the process group",
        "self.process\n            .detach_thread(self.id)" in process,
        failures,
    )
    check(
        "thread owns trap-frame/TLS/signal-mask state",
        all(
            marker in process
            for marker in (
                "user_sp: AtomicUsize",
                "trap_frame: IrqSpinLock<Option<crate::arch::trap::TrapFrame>>",
                "tls: AtomicUsize",
                "blocked_signals: AtomicU64",
            )
        ),
        failures,
    )
    check(
        "process owns planned process-wide resources",
        all(
            marker in process
            for marker in ("FileTable", "SignalState", "Credentials", "FsContext")
        ),
        failures,
    )

    check(
        "UserImage owns Process and Thread",
        "process: Arc<Process>" in user and "thread: Arc<Thread>" in user,
        failures,
    )
    check(
        "UserImage no longer directly owns Box<UserMm>",
        re.search(r"struct\s+UserImage\s*\{[^}]*\bmm\s*:\s*Box<UserMm>", user, re.S)
        is None,
        failures,
    )
    # Rustfmt is free to split field/method chains across lines.  Remove only
    # insignificant whitespace, then require the exact seven M8 UserMm routes
    # to flow through the Process owner.  Also reject the pre-M9 direct field.
    user_compact = re.sub(r"\s+", "", user)
    check(
        "M8 MM operations route through Process/current Thread",
        "self.mm" not in user_compact
        and "current_user_mm()" in user_compact
        and "thread.process().mm()" in user_compact,
        failures,
    )
    check(
        "user entry and stack come from Thread",
        "thread.entry().get()" in user
        and "thread.user_stack_pointer().get()" in user,
        failures,
    )
    check(
        "both per-session and final Process leak gates exist",
        user.count("crate::process::assert_no_leaks();") == 2,
        failures,
    )

    check(
        "Linux syscall numbers are centralized",
        all(
            marker in user
            for marker in (
                "crate::syscall::number::WRITE",
                "crate::syscall::number::EXIT",
                "crate::syscall::number::BRK",
                "crate::syscall::number::MMAP",
            )
        ),
        failures,
    )
    check(
        "Linux errno values are centralized",
        all(
            marker in user
            for marker in (
                "crate::syscall::errno::EBADF",
                "crate::syscall::errno::ENOMEM",
                "crate::syscall::errno::EFAULT",
                "crate::syscall::errno::EINVAL",
                "crate::syscall::errno::ENOSYS",
            )
        ),
        failures,
    )
    check(
        "architecture register decode is centralized",
        "crate::syscall::abi::decode(frame)" in user
        and "frame.gpr[17]" not in user
        and "frame.gpr[11]" not in user[user.find("fn syscall_number"):user.find("fn return_to_kernel")],
        failures,
    )
    check(
        "Linux negative errno range is explicit",
        "MAX_ERRNO: isize = 4095" in syscall
        and "value < 0 && value >= -MAX_ERRNO" in syscall,
        failures,
    )

    check(
        "M8 private-root evidence remains",
        "M8-B3 private-root gate:" in user,
        failures,
    )
    check(
        "M8 demand-paging evidence remains",
        "M8-B4 demand paging/VM gate:" in user,
        failures,
    )
    check(
        "M8 UserMm page-table/ASID implementation is retained",
        "pub fn destroy(&mut self)" in user_mm
        and "pub fn activate_current_cpu(&self)" in user_mm
        and "pub fn deactivate_current_cpu(&self)" in user_mm
        and "enter_cpu_after_local_sync" in user_mm
        and "leave_cpu_after_local_flush" in user_mm,
        failures,
    )
    check(
        "current nightly accepts the GROWSDOWN doc comment",
        re.search(
            r"/// - the expanded range remains inside the configured user range\.\s*\n\s*\n\s*pub fn plan_user_fault",
            user_space,
        )
        is None,
        failures,
    )
    check(
        "RISC-V flush_asid has one inline attribute",
        re.search(r"#\[inline\]\s*#\[inline\]\s*pub fn flush_asid\(", riscv_paging)
        is None
        and len(re.findall(r"#\[inline\]\s*pub fn flush_asid\(", riscv_paging)) == 1,
        failures,
    )
    check(
        "RISC-V SATP switch unsafe block has a concrete SAFETY contract",
        all(
            marker in riscv_paging
            for marker in (
                "// SAFETY: the caller guarantees that `root` and every reachable page-table",
                "this hart's address-space switch is serialized",
                "touches no Rust-managed memory or stack",
            )
        )
        and re.search(
            r"// SAFETY: the caller guarantees.*?unsafe \{",
            riscv_paging,
            re.S,
        )
        is not None,
        failures,
    )
    check(
        "LoongArch per-CPU CSR unsafe block has an adjacent SAFETY contract",
        re.search(
            r"let scratch = cpu;\s*// SAFETY:[^\n]*\n"
            r"(?:\s*//[^\n]*\n)+\s*unsafe \{",
            loongarch_smp,
        )
        is not None,
        failures,
    )

    check(
        "kernel function address uses an explicit pointer cast",
        "verify as *const () as usize" in user
        and "VirtAddr::new(verify as usize)" not in user,
        failures,
    )

    check(
        "TLB user-shootdown documentation has no detached doc block",
        "/// Handles the TLB component of one mailbox batch on the current CPU."
        not in tlb
        and re.search(r"///[^\n]*\n\s*\n\s*/// Executes one synchronous", tlb)
        is None,
        failures,
    )
    check(
        "LoongArch ASID flush documentation has no detached doc block",
        "/// Invalidate the TLB pair containing `address` on the current CPU."
        not in loongarch_paging
        and re.search(r"///[^\n]*\n\s*\n\s*/// Invalidate every non-global", loongarch_paging)
        is None,
        failures,
    )
    check(
        "ASID allocator guards rely on safe deref coercion",
        "ensure_asid_allocator(&mut *allocator)" not in user_mm
        and "ensure_asid_allocator(&mut *slot)" not in user_mm
        and "ensure_asid_allocator(&mut allocator)" in user_mm
        and "ensure_asid_allocator(&mut slot)" in user_mm,
        failures,
    )
    check(
        "retired MM storage is represented by a named TLB-before-free batch",
        all(
            marker in user_mm
            for marker in (
                "struct RetirementBatch",
                "request: Option<PerMmTlbRequest>",
                "backings: Vec<PageAllocation>",
                "page_tables: Vec<PageAllocation>",
                "Result<RetirementBatch, UserMmRuntimeError>",
                "fn finish_retirement(retirement: RetirementBatch)",
            )
        )
        and re.search(
            r"fn retire_range_locked\([^)]*\)\s*->\s*Result\s*<\s*\(",
            user_mm,
            re.S,
        )
        is None,
        failures,
    )

    check(
        "M9-B scheduler boundary is documented",
        all(
            marker in design
            for marker in (
                "M9-B closure",
                "switch_mm_irqs_off()",
                "per-CPU loaded-MM",
                "user-thread migration",
                "m9-completion.md",
            )
        ),
        failures,
    )

    if failures:
        print(f"M9-A audit: FAIL ({len(failures)} failed)")
        return 1

    print("M9-A audit: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
