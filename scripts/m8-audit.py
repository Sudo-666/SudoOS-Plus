#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def read(path: str) -> str:
    target = ROOT / path
    if not target.is_file():
        raise RuntimeError(f"missing required file: {path}")
    return target.read_text(encoding="utf-8")


def check() -> list[str]:
    errors: list[str] = []
    lib = read("mm/src/lib.rs")
    for marker in (
        "mod asid;",
        "mod cpu_mask;",
        "mod user_space;",
        "pub use asid::{",
        "pub use cpu_mask::{",
        "pub use user_space::{",
    ):
        if marker not in lib:
            errors.append(f"mm/src/lib.rs missing {marker!r}")

    for path, markers in {
        "mm/src/asid.rs": (
            "pub struct AsidToken",
            "pub struct AsidAllocator",
            "generation_rolled",
        ),
        "mm/src/cpu_mask.rs": (
            "pub struct CpuMask",
            "pub struct AtomicCpuMask",
            "fetch_or",
            "fetch_and",
        ),
        "mm/src/user_space.rs": (
            "pub struct UserAddressSpace",
            "pub struct PerMmTlbRequest",
            "plan_stack_growth",
            "plan_tlb_request",
            "enter_cpu_after_local_sync",
            "leave_cpu_after_local_flush",
            "assert_inactive_for_destroy",
        ),
    }.items():
        text = read(path)
        for marker in markers:
            if marker not in text:
                errors.append(f"{path} missing {marker!r}")

    # M8-A must not silently pretend M7 already owns private roots.
    user = read("kernel/src/user.rs")
    legacy = (
        "crate::vm::map_user_page",
        "shootdown_kernel_all",
        "M7 keeps local interrupts disabled",
    )
    present = [marker for marker in legacy if marker in user or marker in read("kernel/src/vm.rs")]
    if not present:
        errors.append(
            "M7 global-user-map anchors disappeared; review M8-B baseline instead of applying blindly"
        )

    # Detect accidental weak claims in the design doc.
    doc = read("docs/m8-user-mm.md")
    if re.search(r"M8 (is )?(complete|finished)", doc, re.IGNORECASE):
        errors.append("design document incorrectly claims complete M8 closure")

    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args()
    try:
        errors = check()
    except Exception as error:
        print(f"M8-A audit: FAIL: {error}", file=sys.stderr)
        return 1
    if errors:
        print("M8-A audit: FAIL", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1
    if not args.quiet:
        print("M8-A audit: PASS")
        print("  ASID generation       : present")
        print("  atomic active_cpus    : present")
        print("  re-entry TLB handshake: present")
        print("  per-mm TLB planning   : present")
        print("  bounded stack growth  : present")
        print("  hardware integration  : intentionally deferred to M8-B")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
