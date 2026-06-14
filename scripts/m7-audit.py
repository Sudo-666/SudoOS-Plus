#!/usr/bin/env python3
"""Audit cross-file M7 privilege, syscall, user-copy and teardown invariants."""

from __future__ import annotations

import argparse
import ast
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def check(condition: bool, name: str, detail: str, results: list[dict[str, object]]) -> None:
    status = "PASS" if condition else "FAIL"
    print(f"[{status}] {name}: {detail}")
    results.append({"name": name, "status": status.lower(), "detail": detail})


def require_markers(
    text: str,
    markers: tuple[str, ...],
    name: str,
    results: list[dict[str, object]],
) -> None:
    missing = [marker for marker in markers if marker not in text]
    check(not missing, name, "all required markers present" if not missing else f"missing: {missing}", results)


def assignment_value(source: str, name: str) -> object:
    tree = ast.parse(source, filename="scripts/smoke.py")
    matches = []
    for node in tree.body:
        if not isinstance(node, ast.Assign) or len(node.targets) != 1:
            continue
        target = node.targets[0]
        if isinstance(target, ast.Name) and target.id == name:
            matches.append(node)
    if len(matches) != 1:
        raise RuntimeError(f"expected exactly one {name} assignment, found {len(matches)}")
    return ast.literal_eval(matches[0].value)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()

    required_files = (
        "kernel/src/main.rs",
        "kernel/src/user.rs",
        "kernel/src/trap.rs",
        "kernel/src/vm.rs",
        "kernel/src/user/riscv64.S",
        "kernel/src/user/loongarch64.S",
        "arch/riscv64/src/trap/entry.S",
        "arch/loongarch64/src/trap/entry.S",
        "scripts/smoke.py",
        "Makefile",
    )

    results: list[dict[str, object]] = []
    for relative in required_files:
        check((ROOT / relative).is_file(), f"file:{relative}", "present", results)

    if any(result["status"] == "fail" for result in results):
        return finish(results, args.json)

    main_rs = (ROOT / "kernel/src/main.rs").read_text(encoding="utf-8")
    user_rs = (ROOT / "kernel/src/user.rs").read_text(encoding="utf-8")
    trap_rs = (ROOT / "kernel/src/trap.rs").read_text(encoding="utf-8")
    vm_rs = (ROOT / "kernel/src/vm.rs").read_text(encoding="utf-8")
    riscv_user = (ROOT / "kernel/src/user/riscv64.S").read_text(encoding="utf-8")
    loong_user = (ROOT / "kernel/src/user/loongarch64.S").read_text(encoding="utf-8")
    riscv_trap = (ROOT / "arch/riscv64/src/trap/entry.S").read_text(encoding="utf-8")
    loong_trap = (ROOT / "arch/loongarch64/src/trap/entry.S").read_text(encoding="utf-8")
    smoke = (ROOT / "scripts/smoke.py").read_text(encoding="utf-8")
    makefile = (ROOT / "Makefile").read_text(encoding="utf-8")

    check(
        "user::verify();" in main_rs
        and '#[cfg(debug_assertions)]\n    user::verify();' not in main_rs,
        "release-user-verifier",
        "minimal user mode executes in Debug and Release smoke",
        results,
    )

    require_markers(
        user_rs,
        (
            "const SYS_WRITE: usize = 64;",
            "const SYS_EXIT: usize = 93;",
            "fn copy_from_user(",
            "fn copy_to_user(",
            "checked_add(length)",
            "copy_to_user(USER_CODE, &[0]).is_err()",
            "copy_from_user(USER_DATA + PAGE_SIZE - 1",
            "copy_from_user(usize::MAX - 1",
            "set_syscall_result(frame, -ENOSYS)",
            "LAST_FAULT_ADDRESS.store(address.get()",
            "run_session(",
            "session recycle   : verified (5 runs)",
        ),
        "user-runtime",
        results,
    )

    check(
        "core::ptr::copy_nonoverlapping(source, output.as_mut_ptr()" in user_rs
        and "address as *const" not in user_rs
        and "address as *mut" not in user_rs,
        "checked-user-copy",
        "user virtual addresses are validated before backing-page copy",
        results,
    )

    require_markers(
        user_rs,
        (
            "MappingOptions::user_code()",
            "MappingOptions::user_data()",
            "crate::vm::unmap_user_page(self.stack)",
            "crate::vm::unmap_user_page(self.data)",
            "crate::vm::unmap_user_page(self.code)",
            "copy_from_user(USER_DATA, &mut revoked).is_err()",
        ),
        "mapping-lifecycle",
        results,
    )

    require_markers(
        trap_rs,
        (
            "USER_ECALL if frame.previous_mode_was_user()",
            "ECODE_SYSCALL if frame.previous_mode_was_user()",
            "crate::user::handle_fault(",
            "crate::user::handle_exception(",
        ),
        "trap-routing",
        results,
    )

    require_markers(
        vm_rs,
        (
            "pub struct UserPageMapping",
            "pub fn map_user_page(",
            "pub fn unmap_user_page(",
            "reclaim_empty_tables",
            "shootdown_kernel_all",
        ),
        "vm-lifecycle",
        results,
    )

    require_markers(
        riscv_user,
        (
            "__m7_user_success",
            "__m7_user_unknown_syscall",
            "__m7_user_bad_pointer",
            "__m7_user_write_code",
            "sw zero, 0(t0)",
        ),
        "riscv-user-programs",
        results,
    )
    require_markers(
        loong_user,
        (
            "__m7_user_success",
            "__m7_user_unknown_syscall",
            "__m7_user_bad_pointer",
            "__m7_user_write_code",
            "st.w $r0, $r12, 0",
        ),
        "loongarch-user-programs",
        results,
    )

    check(
        "csrrw sp, sscratch, sp" in riscv_trap
        and "csrw sscratch, zero" in riscv_trap,
        "riscv-trap-stack",
        "sscratch switches user traps to the kernel stack",
        results,
    )
    check(
        "csrwr $r3, 0x30" in loong_trap
        and ".Lloongarch_from_user" in loong_trap
        and ".Lloongarch_return_to_kernel" in loong_trap,
        "loongarch-trap-stack",
        "SAVE0 switches PLV3 traps to the kernel stack",
        results,
    )

    stable = assignment_value(smoke, "STABLE_COMMON_MARKERS")
    phases = assignment_value(smoke, "PHASE_ORDER")
    user_markers = (
        ("user", b"hello user\n"),
        ("user", b"minimal user mode test:"),
    )
    check(
        all(stable.count(marker) == 1 for marker in user_markers),
        "release-smoke-evidence",
        "user markers are stable evidence in both profiles",
        results,
    )
    check(
        phases.count("user") == 1
        and phases.count("final") == 1
        and phases.index("user") + 1 == phases.index("final"),
        "user-phase-order",
        "user evidence is immediately before final success",
        results,
    )

    require_markers(
        makefile,
        (
            ".PHONY: m7-audit",
            ".PHONY: m7-quick",
            ".PHONY: m7-full",
            ".PHONY: m7-soak",
            ".PHONY: m7-release",
            ".PHONY: m7-tag",
        ),
        "m7-release-gates",
        results,
    )

    return finish(results, args.json)


def finish(results: list[dict[str, object]], output: Path | None) -> int:
    status = "pass" if all(result["status"] == "pass" for result in results) else "fail"
    report = {"schema_version": 1, "milestone": "M7-B", "status": status, "checks": results}
    if output is not None:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print("M7 audit report:", output)
    return 0 if status == "pass" else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, SyntaxError, ValueError) as error:
        print(f"m7 audit: error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
