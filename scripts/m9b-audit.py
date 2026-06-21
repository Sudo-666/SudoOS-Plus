#!/usr/bin/env python3
"""Structural audit for M9-B scheduler-integrated user threads and loaded MM."""
from __future__ import annotations

from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]


def read(relative: str) -> str:
    path = ROOT / relative
    if not path.is_file():
        raise SystemExit(f"M9-B audit: missing {relative}")
    return path.read_text(encoding="utf-8")


def compact(text: str) -> str:
    return re.sub(r"\s+", "", text)


def check(name: str, condition: bool, failures: list[str]) -> None:
    print(f"  {'PASS' if condition else 'FAIL'}: {name}")
    if not condition:
        failures.append(name)


def main() -> int:
    task = read("kernel/src/task/mod.rs")
    process = read("kernel/src/process.rs")
    user = read("kernel/src/user.rs")
    user_mm = read("kernel/src/user_mm.rs")
    syscall = read("kernel/src/syscall.rs")
    rv_user = read("kernel/src/user/riscv64.S")
    rv_trap = read("arch/riscv64/src/trap/entry.S")
    la_user = read("kernel/src/user/loongarch64.S")
    design = read("docs/m9-completion.md")
    makefile = read("Makefile")
    smoke = read("scripts/smoke.py")
    task_c = compact(task)
    user_c = compact(user)
    failures: list[str] = []

    check(
        "scheduler has a first-class UserThread kind",
        "UserThread" in task and "TaskKind::UserThread" in task,
        failures,
    )
    check(
        "scheduler Task owns Thread and join completion",
        "user_thread: Option<Arc<crate::process::Thread>>" in task
        and "user_join: Option<Arc<Completion>>" in task
        and "UserTaskHandle" in task,
        failures,
    )
    check(
        "every user task receives a guarded KernelStack",
        "unable to allocate user-thread kernel stack" in task
        and "fresh_task_context(&stack, user_thread_bootstrap)" in task,
        failures,
    )
    check(
        "Thread lifecycle includes transactional scheduler binding",
        all(
            marker in process
            for marker in (
                "THREAD_RUNNABLE",
                "bind_scheduler_task",
                "M9-B Thread task binding changed during rollback",
                "mark_running",
            )
        ),
        failures,
    )
    check(
        "reaper releases scheduler Thread ownership before join completion",
        task_c.find("drop(self.user_thread.take())")
        < task_c.find("join.complete_all()")
        and task_c.find("drop(self.user_thread.take())") >= 0,
        failures,
    )
    bootstrap_start = task.find("unsafe extern \"C\" fn user_thread_bootstrap")
    bootstrap_end = task.find("unsafe extern \"C\" fn kernel_thread_bootstrap", bootstrap_start)
    bootstrap_c = compact(task[bootstrap_start:bootstrap_end])
    check(
        "non-returning user bootstrap drops its scheduler Thread clone",
        "drop(thread);exit_current()" in bootstrap_c,
        failures,
    )
    check(
        "per-CPU loaded MM is a strong Arc",
        "loaded_mm: Option<Arc<crate::user_mm::UserMm>>" in task,
        failures,
    )
    switch_start = task.find("fn switch_mm_irqs_off")
    switch_end = task.find("fn enqueue", switch_start)
    switch = task[switch_start:switch_end]
    switch_c = compact(switch)
    check(
        "switch_mm runs IRQ-off and validates outgoing ownership",
        "assert!(crate::arch::interrupt::are_disabled()" in switch_c
        and "CPUloaded-mmdivergedfromtheoutgoingusertask" in switch_c,
        failures,
    )
    check(
        "switch_mm leaves old MM before entering new MM",
        0 <= switch_c.find(".deactivate_current_cpu()")
        < switch_c.find(".activate_current_cpu()"),
        failures,
    )
    check(
        "all scheduler switch dispositions invoke switch_mm",
        task.count("self.switch_mm_irqs_off(cpu, previous, next);") == 4,
        failures,
    )
    check(
        "same-MM switch avoids unnecessary root churn",
        "Arc::ptr_eq(loaded,incoming)" in switch_c and "return;" in switch_c,
        failures,
    )
    check(
        "same-MM branch does not shadow the incoming TaskId",
        "Some(next))" not in switch and "self.task(next).user_thread" in switch_c,
        failures,
    )
    check(
        "same-MM fast path is a single non-nested condition",
        "iflet(Some(loaded),Some(incoming))=(&loaded_mm,&next_mm)&&Arc::ptr_eq(loaded,incoming)"
        in switch_c,
        failures,
    )
    check(
        "global raw ACTIVE_MM lookup is removed",
        "ACTIVE_MM" not in user_mm
        and "AtomicPtr<UserMm>" not in user_mm
        and all(
            helper not in user_mm
            for helper in (
                "copy_from_active",
                "copy_to_active",
                "resolve_active_fault",
                "active_program_break",
                "set_active_program_break",
                "map_active_anonymous",
                "unmap_active_range",
                "protect_active_range",
            )
        ),
        failures,
    )
    check(
        "syscall/fault memory follows scheduler current Thread",
        "current_user_thread()" in user
        and "current_user_mm()" in user
        and "resolve_user_fault" in user
        and "resolve_active_fault" not in user,
        failures,
    )
    check(
        "scheduled entry validates private root and shared kernel mapping",
        "fnassert_private_hardware_state" not in user_c
        and "mm.root_is_private()" in user_c
        and "mm.assert_hardware_active()" in user_c
        and "mm.kernel_mapping_is_shared" in user_c
        and "run_scheduled_thread" in user,
        failures,
    )
    check(
        "copy_to_user uaccess helper is live code without dead-code allowance",
        "fn copy_to_user(address: usize, input: &[u8]) -> Result<(), ()>" in user
        and '#[allow(dead_code)]\nfn copy_to_user' not in user,
        failures,
    )
    check(
        "brk preserves Linux current-break failure semantics",
        "letcurrent=matchmm.program_break(){Ok(current)=>current,Err(_)=>return-ENOMEM,};" in user_c
        and "Err(_)=>current.get()asisize" in user_c,
        failures,
    )
    check(
        "user mode enables timer/IPI delivery on both architectures",
        "SPIE: enable timer/IPI after sret" in rv_user
        and "PPLV=3, PIE=1" in la_user,
        failures,
    )
    check(
        "Linux sched_yield and exit_group ABI are wired",
        "SCHED_YIELD: usize = 124" in syscall
        and "EXIT_GROUP: usize = 94" in syscall
        and "SYS_SCHED_YIELD" in user
        and "SYS_EXIT | SYS_EXIT_GROUP" in user,
        failures,
    )
    check(
        "sched_yield switches from the current task kernel stack",
        "pub(crate) fn yield_from_user_trap()" in task
        and "crate::task::yield_from_user_trap();" in user
        and "task-stack anchor" in rv_trap,
        failures,
    )
    check(
        "RISC-V user trap restores kernel tp before Rust",
        "sd tp, RISCV_USER_ANCHOR_USER_TP(sp)" in rv_trap
        and "ld tp, RISCV_USER_ANCHOR_KERNEL_TP(sp)" in rv_trap
        and "ld sp, RISCV_USER_ANCHOR_KERNEL_SP(sp)" in rv_trap
        and rv_trap.find("ld tp, RISCV_USER_ANCHOR_KERNEL_TP(sp)")
        < rv_trap.find("call kernel_riscv_trap"),
        failures,
    )
    check(
        "RISC-V trap anchor preserves user tp and migration CPU identity",
        "ld t1, RISCV_TF_PADDING(sp)" in rv_trap
        and "sd t1, 4*8(sp)" in rv_trap
        and "sd tp, RISCV_TF_GUARD(sp)" in rv_trap
        and "csrw sscratch, t1" in rv_trap
        and "addi t2, sp, -RISCV_USER_ANCHOR_SIZE" in rv_user
        and "sd tp, RISCV_USER_ANCHOR_KERNEL_TP(t2)" in rv_user,
        failures,
    )
    check(
        "RISC-V user probe rejects leaked kernel tp",
        "bnez tp, .Lriscv_m9_tp_leaked" in rv_user,
        failures,
    )
    check(
        "scheduler peer is runnable before the user yield probe starts",
        "SCHEDULER_PEER_READY.reinit()" in user
        and "SCHEDULER_PEER_READY.wait()" in user
        and "SCHEDULER_PEER_READY.complete_all()" in user,
        failures,
    )
    check(
        "each sched_yield proves a direct switch away and back",
        "letschedules_before=thread.schedule_count();" in user_c
        and "thread.schedule_count()>schedules_before" in user_c
        and "SCHED_YIELD_SWITCH_COUNT.fetch_add(1,Ordering::AcqRel);" in user_c
        and "SCHED_YIELD_SWITCH_COUNT.load(Ordering::Acquire),8" in user_c,
        failures,
    )
    check(
        "dual-architecture user payload exercises eight sched_yield calls",
        "__m9_user_sched_yield" in rv_user
        and "__m9_user_sched_yield" in la_user
        and "li t0, 8" in rv_user
        and "addi.d $r12, $r0, 8" in la_user,
        failures,
    )
    check(
        "SMP probe pins peer and user task to an explicit CPU",
        "spawn_kernel_thread_on(scheduler_peer,target)" in user_c
        and "spawn_user_thread_on(Arc::clone(&image.thread),target)" in user_c
        and "visited_cpu_mask()" in user,
        failures,
    )
    check(
        "MM teardown waits for scheduler/reaper detachment",
        0 <= user_c.find("task.wait_for_detach()") < user_c.find("image.destroy()"),
        failures,
    )
    check(
        "final gate rejects loaded-MM and user-task leaks",
        "assert_user_mm_quiescent" in task
        and "retained a user task in the scheduler table" in task
        and "retained a user task in the reaper queue" in task
        and "crate::task::assert_user_mm_quiescent();" in user
        and "user_mm_switches()>=18" in user_c,
        failures,
    )
    check(
        "M8 ASID/active_cpus/TLB-before-free core is retained",
        all(
            marker in user_mm
            for marker in (
                "enter_cpu_after_local_sync",
                "leave_cpu_after_local_flush",
                "finish_retirement",
                "shootdown_user_request(request)",
            )
        ),
        failures,
    )
    check(
        "normal verify and smoke gates require M9-B evidence",
        "m9-audit" in makefile
        and "scripts/m9b-audit.py" in makefile
        and "M9-B scheduler/MM gate:" in smoke,
        failures,
    )
    check(
        "completion document records Linux-like MM lifetime contract",
        all(
            marker in design
            for marker in (
                "switch_mm_irqs_off",
                "loaded_mm",
                "TLB-before-free",
                "M10",
            )
        ),
        failures,
    )

    if failures:
        print(f"M9-B audit: FAIL ({len(failures)} failed)", file=sys.stderr)
        return 1
    print("M9-B audit: PASS (33/33)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
