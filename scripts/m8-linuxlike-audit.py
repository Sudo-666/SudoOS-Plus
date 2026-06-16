#!/usr/bin/env python3
"""Audit the accepted Linux-like M8 boundary.

This audit intentionally rejects the abandoned closure design that introduced
per-CPU raw current-mm pointers before Process/Task ownership existed.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


class AuditFailure(RuntimeError):
    pass


def read(repo: Path, relative: str) -> str:
    path = repo / relative
    if not path.is_file():
        raise AuditFailure(f"missing required file: {relative}")
    return path.read_text()


def function_body(text: str, name: str) -> str:
    match = re.search(rf"\bfn\s+{re.escape(name)}\b[^{{]*\{{", text)
    if not match:
        raise AuditFailure(f"missing function: {name}")
    opening = match.end() - 1
    depth = 0
    for index in range(opening, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[opening + 1 : index]
    raise AuditFailure(f"unbalanced function body: {name}")


def check(name: str, condition: bool, failures: list[str]) -> None:
    print(f"  {'PASS' if condition else 'FAIL'}  {name}")
    if not condition:
        failures.append(name)


def main() -> None:
    repo = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()

    user_mm = read(repo, "kernel/src/user_mm.rs")
    user = read(repo, "kernel/src/user.rs")
    tlb = read(repo, "kernel/src/tlb.rs")
    user_space = read(repo, "mm/src/user_space.rs")
    asid = read(repo, "mm/src/asid.rs")
    rv_paging = read(repo, "arch/riscv64/src/memory/paging/mod.rs")
    la_paging = read(repo, "arch/loongarch64/src/memory/paging/mod.rs")
    la_hardware = read(repo, "arch/loongarch64/src/memory/paging/hardware.rs")
    la_smp = read(repo, "arch/loongarch64/src/smp.rs")
    la_entry = read(repo, "arch/loongarch64/src/trap/entry.S")
    la_user = read(repo, "kernel/src/user/loongarch64.S")
    contract = read(repo, "docs/m8-linuxlike-contract.md")

    combined_runtime = user_mm + "\n" + user
    forbidden = (
        "ACTIVE_MMS",
        "active_mm_slot",
        "shootdown_user_after_unlock",
        "bind_current_cpu",
        "unbind_current_cpu",
    )

    finish = function_body(user_mm, "finish_retirement")
    finish_flush = finish.find("crate::tlb::shootdown_user_local(request)")
    finish_free = finish.find("crate::page_alloc::free(")

    run_session = function_body(user, "run_session")
    irq_guard = run_session.find("let _interrupt_guard = crate::context::IrqSaveGuard::new()")
    activate = run_session.find("image.activate_current_cpu()", irq_guard)
    enter = run_session.find("enter_user(", activate)
    deactivate = run_session.find("image.deactivate_current_cpu()", enter)

    publish = function_body(user, "publish")
    unpublish = function_body(user, "unpublish")

    m8_payload = (
        la_user.split("__m8_user_vm:", 1)[1]
        if "__m8_user_vm:" in la_user
        else ""
    )

    failures: list[str] = []

    check(
        "M8 uses one verifier-session ACTIVE_MM binding",
        re.search(r"\bstatic\s+ACTIVE_MM\s*:\s*AtomicPtr\s*<\s*UserMm\s*>", user_mm)
        is not None,
        failures,
    )
    check(
        "abandoned per-CPU raw current-mm closure is absent",
        all(marker not in combined_runtime for marker in forbidden),
        failures,
    )
    check(
        "UserImage strongly owns one UserMm",
        re.search(r"\bmm\s*:\s*Box\s*<\s*UserMm\s*>", user) is not None,
        failures,
    )
    check(
        "publish/unpublish bind exactly the owned mm",
        "self.mm.bind()" in publish and "self.mm.unbind()" in unpublish,
        failures,
    )
    check(
        "private-root round trip is one IRQ-off critical section",
        0 <= irq_guard < activate < enter < deactivate,
        failures,
    )
    check(
        "M8 hardware gate asserts a single active CPU",
        re.search(
            r"assert_eq!\s*\(\s*active\.count\(\)\s*,\s*1\b",
            user_mm,
            flags=re.DOTALL,
        )
        is not None,
        failures,
    )
    check(
        "RISC-V exposes ASID/root switch primitives",
        all(
            marker in rv_paging
            for marker in (
                "maximum_address_space_id",
                "switch_user_address_space",
                "current_address_space_id",
                "flush_asid",
                "flush_asid_page",
            )
        ),
        failures,
    )
    check(
        "LoongArch exposes ASID/root switch primitives",
        all(
            marker in la_paging
            for marker in (
                "maximum_address_space_id",
                "switch_user_address_space",
                "current_address_space_id",
                "flush_asid",
                "flush_asid_page",
            )
        ),
        failures,
    )
    check(
        "ASID allocator tracks generations",
        all(marker in asid for marker in ("AsidAllocator", "generation", "generation_rolled")),
        failures,
    )
    check(
        "UserAddressSpace owns active_cpus and plans per-mm TLB requests",
        all(
            marker in user_space
            for marker in (
                "active_cpus",
                "plan_tlb_request",
                "PerMmTlbRequest",
                "tlb_generation",
            )
        ),
        failures,
    )
    check(
        "remote-capable user shootdown is explicit task-context code",
        "pub fn shootdown_user(" in tlb
        and "MigrationGuard::new()" in function_body(tlb, "shootdown_user")
        and "assert_interrupts_enabled()" in function_body(tlb, "shootdown_user")
        and "assert_task_context()" in function_body(tlb, "shootdown_user"),
        failures,
    )
    check(
        "M8 local shootdown is explicit and fail-closed",
        "pub fn shootdown_user_local(" in tlb
        and "assert_interrupts_disabled()" in function_body(tlb, "shootdown_user_local")
        and "requested & !current_bit" in function_body(tlb, "shootdown_user_local"),
        failures,
    )
    check(
        "M8 fault and mutation paths use the local verifier shootdown",
        user_mm.count("crate::tlb::shootdown_user_local(request)") >= 3,
        failures,
    )
    check(
        "finish_retirement completes TLB invalidation before free",
        0 <= finish_flush < finish_free,
        failures,
    )
    check(
        "demand paging and VM syscalls are integrated",
        all(
            marker in user
            for marker in (
                "SYS_BRK",
                "SYS_MMAP",
                "SYS_MUNMAP",
                "SYS_MPROTECT",
                "__m8_user_vm",
                "demand fault path",
            )
        ),
        failures,
    )
    check(
        "recoverable fault planner covers anonymous and stack growth",
        all(
            marker in user_mm
            for marker in (
                "UserFaultRecovery::Anonymous",
                "UserFaultRecovery::StackGrowth",
                "resolve_user_fault",
            )
        ),
        failures,
    )
    check(
        "LoongArch page invalidation uses global-or-ASID INVTLB op 0x6",
        "const INVTLB_GLOBAL_OR_MATCHING_ASID_AND_VA: usize = 0x6;" in la_hardware
        and re.search(
            r"fn\s+flush_asid_page\b.*?"
            r"operation\s*=\s*const\s+INVTLB_GLOBAL_OR_MATCHING_ASID_AND_VA",
            la_hardware,
            flags=re.DOTALL,
        )
        is not None,
        failures,
    )
    check(
        "obsolete LoongArch INVTLB op 0x5 page constant is absent",
        re.search(
            r"const\s+INVTLB_MATCHING_ASID_AND_VA\s*:\s*usize\s*=\s*0x5",
            la_hardware,
        )
        is None,
        failures,
    )
    check(
        "LoongArch CPU ID is mirrored in KSave3",
        "CSR_PERCPU_ID_SAVE" in la_smp
        and "0x33" in la_smp
        and "csrwr" in la_smp,
        failures,
    )
    check(
        "LoongArch trap entry restores kernel r21 before Rust",
        re.search(r"csrrd\s+\$r21\s*,\s*0x33", la_entry) is not None,
        failures,
    )
    check(
        "LoongArch M8 user payload never uses kernel r21",
        bool(m8_payload) and "$r21" not in m8_payload,
        failures,
    )
    check(
        "M8/M9 ownership boundary is documented",
        all(
            marker in contract
            for marker in (
                "UserImage",
                "Box<UserMm>",
                "Task -> Process -> Arc<UserMm>",
                "switch_mm_irqs_off",
                "M9",
                "MmuGather",
            )
        ),
        failures,
    )
    check(
        "native M8 audits are present",
        all(
            (repo / script).is_file()
            for script in (
                "scripts/m8-audit.py",
                "scripts/m8b3-audit.py",
                "scripts/m8b4-audit.py",
            )
        ),
        failures,
    )

    if failures:
        print("M8 Linux-like audit: FAIL", file=sys.stderr)
        for name in failures:
            print(f"  - {name}", file=sys.stderr)
        raise SystemExit(1)

    print(f"M8 Linux-like audit: PASS ({23 - len(failures)}/23)")


if __name__ == "__main__":
    try:
        main()
    except AuditFailure as error:
        print(f"M8 Linux-like audit: FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
