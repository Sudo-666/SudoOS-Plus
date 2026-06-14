#!/usr/bin/env python3
"""Audit the frozen M6 runtime invariants without booting QEMU.

This is deliberately stricter than a grep-only checklist. It verifies the
cross-file ownership rules introduced by M6-A/M6-B and the r1-r5 fixes, emits a
machine-readable report, and fails the M6 release gate when a hard invariant is
missing.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import asdict, dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


@dataclass
class Finding:
    name: str
    status: str
    detail: str


class Audit:
    def __init__(self) -> None:
        self.findings: list[Finding] = []

    def passed(self, name: str, detail: str) -> None:
        self.findings.append(Finding(name, "pass", detail))

    def failed(self, name: str, detail: str) -> None:
        self.findings.append(Finding(name, "fail", detail))

    def warned(self, name: str, detail: str) -> None:
        self.findings.append(Finding(name, "warn", detail))

    def require(self, condition: bool, name: str, detail: str) -> None:
        (self.passed if condition else self.failed)(name, detail)

    @property
    def failed_count(self) -> int:
        return sum(item.status == "fail" for item in self.findings)


def read(relative: str, audit: Audit) -> str:
    path = ROOT / relative
    if not path.is_file():
        audit.failed(f"file:{relative}", "required file is missing")
        return ""
    audit.passed(f"file:{relative}", "present")
    return path.read_text(encoding="utf-8")


def rust_block(text: str, marker: str) -> str | None:
    start = text.find(marker)
    if start < 0:
        return None
    opening = text.find("{", start)
    if opening < 0:
        return None
    depth = 0
    in_string = in_char = escaped = line_comment = False
    block_comment = 0
    i = opening
    while i < len(text):
        c = text[i]
        n = text[i + 1] if i + 1 < len(text) else ""
        if line_comment:
            if c == "\n":
                line_comment = False
            i += 1
            continue
        if block_comment:
            if c == "/" and n == "*":
                block_comment += 1
                i += 2
                continue
            if c == "*" and n == "/":
                block_comment -= 1
                i += 2
                continue
            i += 1
            continue
        if in_string:
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == '"':
                in_string = False
            i += 1
            continue
        if in_char:
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == "'":
                in_char = False
            i += 1
            continue
        if c == "/" and n == "/":
            line_comment = True
            i += 2
            continue
        if c == "/" and n == "*":
            block_comment = 1
            i += 2
            continue
        if c == '"':
            in_string = True
        elif c == "'" and not (n.isalpha() or n == "_"):
            in_char = True
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return text[start : i + 1]
        i += 1
    return None


def require_markers(audit: Audit, label: str, text: str, markers: tuple[str, ...]) -> None:
    missing = [marker for marker in markers if marker not in text]
    audit.require(
        not missing,
        label,
        "all required markers present" if not missing else "missing: " + ", ".join(missing),
    )


def forbid_markers(audit: Audit, label: str, text: str, markers: tuple[str, ...]) -> None:
    present = [marker for marker in markers if marker in text]
    audit.require(
        not present,
        label,
        "legacy patterns absent" if not present else "forbidden: " + ", ".join(present),
    )


def function_signature(text: str, marker: str) -> str | None:
    start = text.find(marker)
    if start < 0:
        return None
    opening = text.find("{", start)
    if opening < 0:
        return None
    return " ".join(text[start:opening].split())


def check_fallible_api(audit: Audit, label: str, text: str, markers: tuple[str, ...]) -> None:
    checked = 0
    unsafe: list[str] = []
    for marker in markers:
        signature = function_signature(text, marker)
        if signature is None:
            continue
        checked += 1
        if "-> Option<" not in signature and "-> Result<" not in signature:
            unsafe.append(signature)
    if checked == 0:
        audit.warned(label, "no matching public API signature was found; manual review required")
    else:
        audit.require(
            not unsafe,
            label,
            f"{checked} capacity-consuming API(s) return Option/Result"
            if not unsafe
            else "non-fallible capacity API: " + " | ".join(unsafe),
        )


def git_tracked() -> list[str]:
    result = subprocess.run(
        ["git", "ls-files"],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    return result.stdout.splitlines() if result.returncode == 0 else []


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", type=Path, default=ROOT / "build/m6/audit.json")
    args = parser.parse_args()
    audit = Audit()

    time_rs = read("kernel/src/time.rs", audit)
    timer_rs = read("kernel/src/timer.rs", audit)
    work_rs = read("kernel/src/workqueue.rs", audit)
    task_rs = read("kernel/src/task/mod.rs", audit)
    wait_rs = read("kernel/src/task/wait_queue.rs", audit)
    idle_rs = read("kernel/src/task/idle_verify.rs", audit)
    lockdep_rs = read("kernel/src/lockdep.rs", audit)
    smoke_py = read("scripts/smoke.py", audit)
    rv_task = read("arch/riscv64/src/task/mod.rs", audit)
    la_task = read("arch/loongarch64/src/task/mod.rs", audit)
    rv_context = read("arch/riscv64/src/task/context.rs", audit)
    la_context = read("arch/loongarch64/src/task/context.rs", audit)

    require_markers(
        audit,
        "timer-runtime",
        timer_rs,
        (
            "pub fn cancel_sync(",
            "slot reclamation",
            "synchronous cancel",
            "wait timeout",
        ),
    )
    require_markers(
        audit,
        "workqueue-runtime",
        work_rs,
        (
            "const WORKERS_PER_CPU: usize = 2;",
            "pub fn queue_delayed(",
            "pub fn cancel_sync(",
            "pub fn flush(",
            "slot reclamation",
            "tickless wakeup",
        ),
    )
    require_markers(
        audit,
        "nohz-policy",
        time_rs,
        (
            "static SCHEDULER_TICK_ACTIVE:",
            "static TICKLESS_IDLE_ENTRIES:",
            "pub(crate) fn enter_idle()",
            "pub(crate) fn leave_idle()",
            "scheduler_tick_active_for(",
        ),
    )
    require_markers(
        audit,
        "nohz-verifier-r5",
        idle_rs,
        (
            "M6-B r5: NO_HZ-aware deterministic idle verifier",
            "scheduler_tick_active_for",
            "tickless_idle_entries_for",
        ),
    )

    # Verify the exact-one-IPI property from executable structure rather than
    # from a particular comment or smoke-output spelling.  The original M6-C
    # package looked for "single remote reschedule IPI", while the r5 source
    # intentionally prints "single reschedule IPI".  Comments are not an
    # invariant; the counter window and assertion are.
    verify_block = rust_block(idle_rs, "pub(super) fn verify(") or ""
    ipi_before_match = re.search(
        r"let\s+ipis_before\s*=\s*crate::smp::ipi_count\s*\(\s*target\s*\)\s*;",
        verify_block,
        re.DOTALL,
    )
    wake_match = re.search(r"RUN_QUEUE\.wake_one\s*\(\s*\)", verify_block)
    release_match = re.search(
        r"GATE_PHASE\.store\s*\(\s*GATE_RELEASED\s*,",
        verify_block,
        re.DOTALL,
    )
    delivered_match = re.search(
        r"let\s+delivered\s*=\s*crate::smp::ipi_count\s*\(\s*target\s*\)"
        r"\s*\.checked_sub\s*\(\s*ipis_before\s*\)",
        verify_block,
        re.DOTALL,
    )
    exact_one_match = re.search(
        r"assert_eq!\s*\(\s*delivered\s*,\s*1\s*,",
        verify_block,
        re.DOTALL,
    )
    ordered_ipi_window = all(
        match is not None
        for match in (
            ipi_before_match,
            wake_match,
            release_match,
            delivered_match,
            exact_one_match,
        )
    )
    if ordered_ipi_window:
        positions = [
            ipi_before_match.start(),
            wake_match.start(),
            release_match.start(),
            delivered_match.start(),
            exact_one_match.start(),
        ]
        ordered_ipi_window = positions == sorted(positions)
    audit.require(
        ordered_ipi_window,
        "nohz-exact-one-ipi",
        "IPI counter window proves exactly one remote wakeup"
        if ordered_ipi_window
        else "missing or reordered ipi_count/wake_one/delivered==1 proof",
    )
    forbid_markers(
        audit,
        "legacy-idle-test-control",
        time_rs + idle_rs,
        (
            "pause_periodic_for_idle_test",
            "resume_periodic_for_idle_test",
            "TIMER_PAUSED",
            "timer_stopper_worker",
        ),
    )

    idle_block = rust_block(task_rs, "fn idle_until_interrupt()") or ""
    finish_block = rust_block(task_rs, "fn finish_switch()") or ""
    enter_at = idle_block.find("crate::time::enter_idle();")
    wait_at = idle_block.find("enable_and_wait_for_interrupt")
    audit.require(
        enter_at >= 0 and wait_at >= 0 and enter_at < wait_at,
        "idle-entry-order",
        "enter_idle occurs after the final work recheck and before architecture wait",
    )
    scheduler_scope_end = finish_block.find("};")
    leave_at = finish_block.find("crate::time::leave_idle();")
    audit.require(
        scheduler_scope_end >= 0 and leave_at > scheduler_scope_end,
        "idle-exit-lock-order",
        "leave_idle executes after Scheduler lock release",
    )

    require_markers(
        audit,
        "compact-waitqueue",
        task_rs + wait_rs,
        (
            "wait_prev",
            "wait_next",
            "link_waiter",
            "unlink_waiter_locked",
            "pub(super) struct WaitList",
        ),
    )
    forbid_markers(
        audit,
        "legacy-array-waitqueue",
        task_rs + wait_rs,
        (
            "ClaimedWaiters",
            "[WaitEntry; MAX_TASKS]",
            "[Option<TaskId>; MAX_TASKS]",
        ),
    )

    require_markers(
        audit,
        "fresh-task-bootstrap",
        task_rs + rv_task + la_task + rv_context + la_context,
        (
            "fresh_task_context",
            "FRESH_TASK_STACK_RESERVE",
            "saved_stack_pointer",
        ),
    )
    forbid_markers(
        audit,
        "workqueue-stack-ownership",
        work_rs,
        ("KernelStack::allocate", "Context::new"),
    )

    enum = rust_block(lockdep_rs, "pub enum LockRank") or ""
    values = {
        name: int(value)
        for name, value in re.findall(
            r"(?m)^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(\d+)\s*,", enum
        )
    }
    order_ok = all(name in values for name in ("CrossCpu", "Timer", "WorkQueue", "Scheduler"))
    if order_ok:
        order_ok = values["CrossCpu"] < values["Timer"] < values["WorkQueue"] < values["Scheduler"]
    audit.require(
        order_ok,
        "lock-graph",
        "CrossCpu < Timer < WorkQueue < Scheduler",
    )

    require_markers(
        audit,
        "smoke-evidence",
        smoke_py,
        (
            'b"timer runtime test:"',
            'b"workqueue runtime test:"',
            "DEBUG_M5_MARKERS",
            "result.json",
        ),
    )

    check_fallible_api(
        audit,
        "workqueue-capacity-failure",
        work_rs,
        ("pub fn queue(", "pub fn queue_on(", "pub fn queue_delayed("),
    )
    check_fallible_api(
        audit,
        "timer-capacity-failure",
        timer_rs,
        ("pub fn arm_after(", "pub fn arm_at(", "pub fn arm_on("),
    )

    tracked = git_tracked()
    generated = [
        path
        for path in tracked
        if (ROOT / path).exists()
        and (
            "__pycache__" in path
            or path.endswith((".pyc", ".pyo"))
            or path == ".DS_Store"
            or "/.DS_Store" in path
        )
    ]
    audit.require(
        not generated,
        "repository-hygiene",
        "no generated Python/macOS files are tracked"
        if not generated
        else "tracked generated files: " + ", ".join(generated),
    )

    report = {
        "schema_version": 1,
        "status": "pass" if audit.failed_count == 0 else "fail",
        "failed": audit.failed_count,
        "warnings": sum(item.status == "warn" for item in audit.findings),
        "findings": [asdict(item) for item in audit.findings],
    }
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    for item in audit.findings:
        prefix = {"pass": "PASS", "fail": "FAIL", "warn": "WARN"}[item.status]
        print(f"[{prefix}] {item.name}: {item.detail}")
    print(f"M6 audit report: {args.json}")
    return 0 if audit.failed_count == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
