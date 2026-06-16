#!/usr/bin/env python3
"""M8-B3 R2 symbol hotfix audit."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(relative: str) -> str:
    path = ROOT / relative
    if not path.is_file():
        raise RuntimeError(f"missing required file: {relative}")
    return path.read_text(encoding="utf-8")


def main() -> int:
    try:
        runtime = read("kernel/src/runtime_page_table.rs")
        user_mm = read("kernel/src/user_mm.rs")

        old_runtime = "crate::arch::memory::layout::USER.contains(page.start_address())"
        fixed_runtime = "crate::arch::memory::layout::USER_RANGE.contains(page.start_address())"
        old_constructor = "UserAddressSpace::new(crate::arch::memory::layout::USER, asid)"
        fixed_constructor = "UserAddressSpace::new(crate::arch::memory::layout::USER_RANGE, asid)"

        if old_runtime in runtime or runtime.count(fixed_runtime) != 2:
            raise RuntimeError("runtime page-table user-range checks are not exactly USER_RANGE")
        if old_constructor in user_mm or user_mm.count(fixed_constructor) != 1:
            raise RuntimeError("UserAddressSpace constructor is not using USER_RANGE")
        if "let area = *state\n            .core\n            .layout()" in user_mm:
            raise RuntimeError("find_area() result is still incorrectly dereferenced")
        if "let area = state\n            .core\n            .layout()\n            .find_area(address)" not in user_mm:
            raise RuntimeError("reviewed VmArea value lookup is missing")
    except RuntimeError as error:
        print(f"M8-B3 R2 audit: FAIL: {error}", file=sys.stderr)
        return 1

    print("M8-B3 R2 audit: PASS")
    print("  USER_RANGE symbol : exact")
    print("  VmArea semantics  : by value")
    print("  arch layout alias : unnecessary")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
