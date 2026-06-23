#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MEMORY_RS = ROOT / "kernel" / "src" / "memory.rs"


def fail(name: str, detail: str = "") -> tuple[str, bool, str]:
    return (name, False, detail)


def ok(name: str, detail: str = "") -> tuple[str, bool, str]:
    return (name, True, detail)


def find_matching_brace(src: str, open_idx: int) -> int:
    if open_idx < 0 or open_idx >= len(src) or src[open_idx] != "{":
        raise ValueError("find_matching_brace called without an opening brace")
    depth = 0
    i = open_idx
    in_line_comment = False
    in_block_comment = False
    in_string = False
    in_char = False
    escaped = False
    while i < len(src):
        ch = src[i]
        nxt = src[i + 1] if i + 1 < len(src) else ""
        if in_line_comment:
            if ch == "\n":
                in_line_comment = False
            i += 1
            continue
        if in_block_comment:
            if ch == "*" and nxt == "/":
                in_block_comment = False
                i += 2
                continue
            i += 1
            continue
        if in_string:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_string = False
            i += 1
            continue
        if in_char:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == "'":
                in_char = False
            i += 1
            continue
        if ch == "/" and nxt == "/":
            in_line_comment = True
            i += 2
            continue
        if ch == "/" and nxt == "*":
            in_block_comment = True
            i += 2
            continue
        if ch == '"':
            in_string = True
            i += 1
            continue
        if ch == "'":
            in_char = True
            i += 1
            continue
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    raise ValueError("unmatched brace")


def extract_function(src: str, name: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(name) + r"(?:\s*<[^>{}]*>)?\s*\(", src)
    if not m:
        raise ValueError(f"function {name} not found")
    open_idx = src.find("{", m.end())
    close_idx = find_matching_brace(src, open_idx)
    return src[open_idx + 1:close_idx]


def extract_cfg_block(src: str, start: int, cfg_regex: str) -> tuple[int, int, str]:
    m = re.search(cfg_regex, src[start:], flags=re.S)
    if not m:
        raise ValueError(f"cfg block not found: {cfg_regex}")
    cfg_start = start + m.start()
    after_attr = start + m.end()
    open_idx = src.find("{", after_attr)
    if open_idx < 0:
        raise ValueError("cfg block opening brace not found")
    close_idx = find_matching_brace(src, open_idx)
    return open_idx, close_idx, src[open_idx + 1:close_idx]


def first_pos(src: str, patterns: list[str]) -> int | None:
    positions: list[int] = []
    for pat in patterns:
        m = re.search(pat, src, flags=re.S)
        if m:
            positions.append(m.start())
    return min(positions) if positions else None


def main() -> int:
    src = MEMORY_RS.read_text(encoding="utf-8")
    checks: list[tuple[str, bool, str]] = []
    try:
        body = extract_function(src, "initialize_page_allocator")
    except Exception as e:
        checks.append(fail("initialize_page_allocator found", str(e)))
        return report(checks)

    summary_pos = body.find("physical page allocator")
    if summary_pos < 0:
        checks.append(fail("allocator summary anchor found"))
        return report(checks)
    checks.append(ok("allocator summary anchor found"))

    try:
        rv_open, rv_close, rv_block = extract_cfg_block(
            body,
            summary_pos,
            r"#\s*\[\s*cfg\s*\(\s*target_arch\s*=\s*\"riscv64\"\s*\)\s*\]",
        )
        checks.append(ok("RISC-V allocator block is cfg-gated"))
    except Exception as e:
        checks.append(fail("RISC-V allocator block is cfg-gated", str(e)))
        return report(checks)

    install_patterns = [
        r"page_alloc\s*::\s*install_boot\s*\(",
        r"page_alloc\s*::\s*install\s*\(",
    ]
    rv_install = first_pos(rv_block, install_patterns)
    if rv_install is None:
        checks.append(fail("RISC-V allocator is installed in RISC-V cfg block"))
    else:
        checks.append(ok("RISC-V allocator is installed in RISC-V cfg block"))

    zone_patterns = [r"zone_present_pages\s*\(", r"zone_free_pages\s*\("]
    rv_zone = first_pos(rv_block, zone_patterns)
    if rv_zone is None:
        checks.append(ok("zone summary gated away from riscv before install"))
    else:
        checks.append(fail("zone summary gated away from riscv before install"))

    if rv_install is not None and (rv_zone is None or rv_zone > rv_install):
        checks.append(ok("riscv verbose zone summary not required before install"))
    else:
        checks.append(fail("riscv verbose zone summary not required before install"))

    # Anything between the allocator heading and the RISC-V cfg block is shared
    # pre-install code. It must not query verbose per-zone counters either.
    shared_pre_rv = body[summary_pos:rv_open]
    if first_pos(shared_pre_rv, zone_patterns) is None:
        checks.append(ok("shared pre-install summary has no zone counters"))
    else:
        checks.append(fail("shared pre-install summary has no zone counters"))

    try:
        _nr_open, _nr_close, non_rv_block = extract_cfg_block(
            body,
            rv_close + 1,
            r"#\s*\[\s*cfg\s*\(\s*not\s*\(\s*target_arch\s*=\s*\"riscv64\"\s*\)\s*\)\s*\]",
        )
        checks.append(ok("non-RISC-V verbose zone block is cfg-gated"))
        if first_pos(non_rv_block, zone_patterns) is not None:
            checks.append(ok("non-RISC-V keeps verbose zone summary"))
        else:
            checks.append(fail("non-RISC-V keeps verbose zone summary"))
    except Exception as e:
        checks.append(fail("non-RISC-V verbose zone block is cfg-gated", str(e)))

    if "release_early_ranges_to_buddy_chunked" in body:
        checks.append(ok("chunked early-to-buddy handoff retained"))
    else:
        checks.append(fail("chunked early-to-buddy handoff retained"))

    return report(checks)


def report(checks: list[tuple[str, bool, str]]) -> int:
    passed = sum(1 for _, good, _ in checks if good)
    failed = len(checks) - passed
    print(f"oscomp-riscv-allocator-install-first-audit: PASS={passed}, FAIL={failed}")
    for name, good, detail in checks:
        status = "PASS" if good else "FAIL"
        extra = f" — {detail}" if detail else ""
        print(f"  {status}: {name}{extra}")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
