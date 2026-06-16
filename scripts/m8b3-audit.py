#!/usr/bin/env python3
"""Structural audit for the M8-B3 private user-root hardware gate."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class AuditError(RuntimeError):
    pass


def text(relative: str) -> str:
    path = ROOT / relative
    if not path.is_file():
        raise AuditError(f"missing required file: {relative}")
    return path.read_text(encoding="utf-8")


def require(relative: str, markers: tuple[str, ...]) -> str:
    source = text(relative)
    missing = [marker for marker in markers if marker not in source]
    if missing:
        raise AuditError(f"{relative} is missing markers: {missing}")
    return source


def forbid(relative: str, markers: tuple[str, ...]) -> None:
    source = text(relative)
    found = [marker for marker in markers if marker in source]
    if found:
        raise AuditError(f"{relative} contains forbidden B3 markers: {found}")


def main() -> int:
    try:
        runtime = require(
            "kernel/src/runtime_page_table.rs",
            (
                "root_owner: Option<PageAllocation>",
                "pub fn new_user(kernel: &Self)",
                "KernelMappingMutationDenied",
                "pub fn release_empty(&mut self)",
                "SHARED_KERNEL_ROOT_BORROWERS",
                "shared_kernel_tables_are_borrowed()",
                "Copy only the shared",
                "private user root only in PGDL",
            ),
        )
        if "pub fn release_empty(self)" in runtime:
            raise AuditError("user-root release still consumes ownership on failure")

        require(
            "kernel/src/vm.rs",
            (
                "pub(crate) fn create_user_page_table()",
                "pub(crate) fn synchronize_user_page_table(",
                "pub(crate) fn kernel_page_table_root()",
                "pub(crate) unsafe fn activate_user_page_table(",
                "pub(crate) unsafe fn activate_kernel_page_table()",
            ),
        )
        require(
            "mm/src/asid.rs",
            (
                "pub const fn next_allocation_rolls_generation",
                "reports_the_rollover_boundary_before_reusing_an_id",
            ),
        )
        require(
            "mm/src/user_space.rs",
            (
                "pub enum UserFaultPlan",
                "pub fn plan_user_fault(",
                "pub fn plan_post_install_tlb(",
                "enter_cpu_after_local_sync",
                "leave_cpu_after_local_flush",
            ),
        )
        require(
            "arch/riscv64/src/memory/paging/mod.rs",
            (
                "pub fn maximum_address_space_id() -> u16",
                "pub unsafe fn switch_user_address_space(",
                "csrw satp",
                "pub fn current_lower_root()",
                "pub fn current_address_space_id()",
                "pub fn flush_asid(",
            ),
        )
        require(
            "arch/loongarch64/src/memory/paging/mod.rs",
            (
                "pub fn maximum_address_space_id() -> u16",
                "pub unsafe fn switch_user_address_space(",
                "write_switch_csr::<CSR_PGDL>",
                "write_switch_csr::<CSR_ASID>",
                "pub fn current_upper_root()",
                "flush_asid,",
            ),
        )
        user_mm = require(
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
                "AsidRolloverInProgress",
                "AsidRolloverWithLiveMms",
                "ASID_ROLLOVER_IN_PROGRESS",
                "impl Drop for UserMm",
                "FaultAccess::Write => flags.is_writable()",
            ),
        )
        for forbidden in ("handle_active_fault", "pub fn handle_fault(", "UserFaultPlan"):
            if forbidden in user_mm:
                raise AuditError(
                    "M8-B3 wired demand-fault execution into the hardware-root gate: "
                    f"{forbidden}"
                )

        user = require(
            "kernel/src/user.rs",
            (
                "M8-B3 private-root gate:",
                "session recycle   : verified (5 runs)",
                "demand fault path : intentionally deferred",
                "image.activate_current_cpu();",
                "image.deactivate_current_cpu();",
                "return_to_kernel(frame, -EFAULT);",
            ),
        )
        for forbidden in (
            "crate::vm::map_user_page",
            "crate::vm::unmap_user_page",
            "handle_active_fault",
            "__m8_user_demand_data",
            "__m8_user_grow_stack",
        ):
            if forbidden in user:
                raise AuditError(f"kernel/src/user.rs still contains mixed-scope code: {forbidden}")

        require("kernel/src/main.rs", ("mod user;", "mod user_mm;"))
        require(
            "kernel/src/tlb.rs",
            (
                "pub fn shootdown_user(request: PerMmTlbRequest)",
                "M8-B1 per-mm TLB test:",
            ),
        )
    except AuditError as error:
        print(f"M8-B3 audit: FAIL: {error}", file=sys.stderr)
        return 1

    print("M8-B3 audit: PASS")
    print("  private root ownership : present")
    print("  shared-kernel lifetime : pinned while borrowed")
    print("  RISC-V satp + ASID     : present")
    print("  LoongArch PGDL + ASID  : present")
    print("  active_cpus handshake  : present")
    print("  ASID rollover gate     : present")
    print("  explicit page reclaim  : present")
    print("  M7 fault termination   : preserved")
    print("  B2 fault planner       : retained, not wired")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
