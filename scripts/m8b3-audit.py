#!/usr/bin/env python3
"""Structural audit for M8-B3 invariants after later M8 integration gates."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def source(relative: str) -> str:
    path = ROOT / relative
    if not path.is_file():
        raise RuntimeError(f"missing required file: {relative}")
    return path.read_text(encoding="utf-8")


def require(relative: str, markers: tuple[str, ...]) -> None:
    text = source(relative)
    missing = [marker for marker in markers if marker not in text]
    if missing:
        raise RuntimeError(f"{relative} is missing markers: {missing}")


def main() -> int:
    try:
        require(
            "kernel/src/runtime_page_table.rs",
            (
                "root_owner: Option<PageAllocation>",
                "pub fn new_user(kernel: &Self)",
                "KernelMappingMutationDenied",
                "pub fn release_empty(&mut self)",
                "SHARED_KERNEL_ROOT_BORROWERS",
                "shared_kernel_tables_are_borrowed()",
            ),
        )
        require(
            "kernel/src/vm.rs",
            (
                "pub(crate) fn create_user_page_table()",
                "pub(crate) fn synchronize_user_page_table(",
                "pub(crate) unsafe fn activate_user_page_table(",
                "pub(crate) unsafe fn activate_kernel_page_table()",
            ),
        )
        require(
            "kernel/src/user_mm.rs",
            (
                "pub struct UserMm",
                "pub fn populate_page(",
                "pub fn activate_current_cpu(",
                "enter_cpu_after_local_sync",
                "pub fn deactivate_current_cpu(",
                "leave_cpu_after_local_flush",
                "pub fn assert_hardware_active(",
                "pub fn destroy(&mut self)",
                "page_table.release_empty()?",
                "ASID_ROLLOVER_IN_PROGRESS",
                "impl Drop for UserMm",
            ),
        )
        require(
            "kernel/src/user.rs",
            (
                "M8-B3 private-root gate:",
                "session recycle   : verified (5 runs)",
                "private user root : verified",
                "kernel root return: verified",
                "run_scheduled_thread",
            ),
        )
        require(
            "kernel/src/task/mod.rs",
            (
                "fn switch_mm_irqs_off(",
                "loaded_mm: Option<Arc<crate::user_mm::UserMm>>",
                ".activate_current_cpu()",
                ".deactivate_current_cpu()",
            ),
        )
        require(
            "kernel/src/tlb.rs",
            (
                "pub fn shootdown_user(request: PerMmTlbRequest)",
                "M8-B1 per-mm TLB test:",
            ),
        )
    except RuntimeError as error:
        print(f"M8-B3 audit: FAIL: {error}", file=sys.stderr)
        return 1

    print("M8-B3 audit: PASS")
    print("  private root ownership : retained")
    print("  shared-kernel lifetime : retained")
    print("  dual-arch root switch  : retained")
    print("  active_cpus handshake  : retained")
    print("  ASID rollover gate     : retained")
    print("  explicit page reclaim  : retained")
    print("  later fault integration: accepted")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
