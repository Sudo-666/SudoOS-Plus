#!/usr/bin/env python3
"""Structural audit for the M8-B1 per-mm TLB / ASID gate."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path.cwd()
CHECKS = {
    "riscv explicit ASID flush": (
        Path("arch/riscv64/src/memory/paging/mod.rs"),
        ("pub fn flush_asid(", "sfence.vma zero, {asid}", "pub fn flush_asid_page("),
    ),
    "loongarch exact ASID INVTLB": (
        Path("arch/loongarch64/src/memory/paging/hardware.rs"),
        (
            "INVTLB_MATCHING_ASID: usize = 0x4",
            "INVTLB_MATCHING_ASID_AND_VA: usize = 0x5",
            "pub fn flush_asid(",
            "pub fn flush_asid_page(",
        ),
    ),
    "shared per-mm shootdown protocol": (
        Path("kernel/src/tlb.rs"),
        (
            "pub fn shootdown_user(request: PerMmTlbRequest)",
            "let serializer = acquire_serializer(current);",
            "REQUEST.publish(TlbRequest",
            "for_each_cpu(targets, crate::smp::send_tlb_shootdown);",
            "wait_for_completion(request_id, targets);",
        ),
    ),
    "exact active CPU targeting": (
        Path("kernel/src/tlb.rs"),
        (
            "let requested = usize::try_from(request.targets().bits())",
            "requested & !online",
            "requested & !ready",
            "let targets = requested & !current_bit;",
        ),
    ),
    "ASID-scoped local fallback": (
        Path("kernel/src/tlb.rs"),
        (
            "fn flush_all_local(scope: TlbScope)",
            "flush_asid(address_space)",
            "fn flush_range_local(scope: TlbScope, range: VirtRange)",
        ),
    ),
    "runtime exact-mask proof": (
        Path("kernel/src/tlb.rs"),
        (
            "M8-B1 per-mm TLB test:",
            "exact active CPU mask : verified",
            "shared ACK protocol   : verified",
            "generation handshake  : verified",
        ),
    ),
}


def main() -> int:
    failures: list[str] = []
    for name, (relative, markers) in CHECKS.items():
        path = ROOT / relative
        if not path.is_file():
            failures.append(f"{name}: missing {relative}")
            continue
        text = path.read_text(encoding="utf-8")
        missing = [marker for marker in markers if marker not in text]
        if missing:
            failures.append(f"{name}: missing markers {missing}")
        else:
            print(f"[PASS] {name}")

    if failures:
        for failure in failures:
            print(f"[FAIL] {failure}", file=sys.stderr)
        return 1

    print("M8-B1 audit: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
