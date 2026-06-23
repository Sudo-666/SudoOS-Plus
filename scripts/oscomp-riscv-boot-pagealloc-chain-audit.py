#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8")


def find_matching_brace(src: str, open_idx: int) -> int:
    depth = 0
    i = open_idx
    n = len(src)
    in_line_comment = False
    in_block_comment = 0
    in_str = False
    in_char = False
    raw_hashes = None
    escape = False
    while i < n:
        ch = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if in_line_comment:
            if ch == "\n":
                in_line_comment = False
            i += 1
            continue
        if in_block_comment:
            if ch == "/" and nxt == "*":
                in_block_comment += 1
                i += 2
                continue
            if ch == "*" and nxt == "/":
                in_block_comment -= 1
                i += 2
                continue
            i += 1
            continue
        if raw_hashes is not None:
            if ch == '"' and src.startswith("#" * raw_hashes, i + 1):
                i += 1 + raw_hashes
                raw_hashes = None
            else:
                i += 1
            continue
        if in_str:
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == '"':
                in_str = False
            i += 1
            continue
        if in_char:
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == "'":
                in_char = False
            i += 1
            continue
        if ch == "/" and nxt == "/":
            in_line_comment = True
            i += 2
            continue
        if ch == "/" and nxt == "*":
            in_block_comment = 1
            i += 2
            continue
        if ch == "r":
            j = i + 1
            hashes = 0
            while j < n and src[j] == "#":
                hashes += 1
                j += 1
            if hashes and j < n and src[j] == '"':
                raw_hashes = hashes
                i = j + 1
                continue
        if ch == '"':
            in_str = True
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
    raise RuntimeError("unmatched brace")


def extract_fn(src: str, name: str) -> str:
    m = re.search(r"\b(?:pub\s+)?(?:unsafe\s+)?fn\s+" + re.escape(name) + r"(?:\s*<[^>{;]*>)?\s*\(", src)
    if not m:
        raise RuntimeError(f"function {name} not found")
    open_idx = src.find("{", m.end())
    if open_idx < 0:
        raise RuntimeError(f"function {name} has no body")
    close_idx = find_matching_brace(src, open_idx)
    return src[m.start():close_idx + 1]


def extract_cfg_block(src: str, cfg: str) -> str:
    marker = f"#[cfg({cfg})]"
    start = src.find(marker)
    if start < 0:
        raise RuntimeError(f"cfg block {cfg} not found")
    open_idx = src.find("{", start)
    if open_idx < 0:
        raise RuntimeError(f"cfg block {cfg} has no body")
    close_idx = find_matching_brace(src, open_idx)
    return src[start:close_idx + 1]


def main() -> int:
    memory = read("kernel/src/memory.rs")
    page_alloc = read("kernel/src/page_alloc.rs")
    irq_lock = read("kernel/src/irq_lock.rs")
    spin_lock = read("sync/src/spin_lock.rs")

    results: list[tuple[bool, str]] = []
    def check(cond: bool, msg: str) -> None:
        results.append((cond, msg))

    init_fn = extract_fn(memory, "initialize_page_allocator")
    riscv_block = extract_cfg_block(init_fn, 'target_arch = "riscv64"')

    check("unsafe" in riscv_block and "page_alloc::install_boot(page_allocator)" in riscv_block,
          "RISC-V uses boot-only page allocator install")
    check("page_alloc::install(page_allocator)" not in riscv_block,
          "RISC-V allocator install avoids runtime IRQ lock")
    check("zone_present_pages" not in riscv_block and "zone_free_pages" not in riscv_block,
          "RISC-V allocator summary avoids verbose zone walk before/around install")
    check("page_alloc::is_initialized()" not in riscv_block,
          "RISC-V allocator init path avoids runtime is_initialized lock")
    # Post-install boot reread was intentionally removed after tracing proved install_boot returns.
    check("is_initialized_boot" not in riscv_block,
          "RISC-V allocator init avoids post-install global reread")

    check("pub unsafe fn install_boot" in page_alloc and "PAGE_ALLOCATOR.get_mut_unchecked()" in page_alloc,
          "page_alloc exposes boot-only global publish path")
    check("pub unsafe fn get_mut_unchecked" in irq_lock and ".get_mut_unchecked()" in irq_lock,
          "IrqSpinLock forwards boot-only mutable access")
    check("pub unsafe fn get_mut_unchecked" in spin_lock and "UnsafeCell" in spin_lock,
          "SpinLock exposes explicit boot-only mutable access")
    check("riscv page_alloc install_boot:" not in memory + page_alloc,
          "temporary RISC-V install_boot trace removed")

    ok = sum(1 for passed, _ in results if passed)
    fail = len(results) - ok
    for passed, msg in results:
        print(("PASS" if passed else "FAIL") + f": {msg}")
    print(f"oscomp-riscv-boot-pagealloc-chain-audit: PASS={ok}, FAIL={fail}")
    return 0 if fail == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
