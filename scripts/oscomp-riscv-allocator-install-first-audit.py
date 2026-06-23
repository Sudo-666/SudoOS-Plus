#!/usr/bin/env python3
from pathlib import Path
import re
import sys

root = Path(__file__).resolve().parents[1]
src = (root / "kernel/src/memory.rs").read_text()

checks = []

def add(name, ok):
    checks.append((name, bool(ok)))

def find_matching_brace(text, open_pos):
    depth = 0
    i = open_pos
    in_line = in_block = in_str = False
    escape = False
    while i < len(text):
        ch = text[i]
        nxt = text[i + 1] if i + 1 < len(text) else ""
        if in_line:
            if ch == "\n": in_line = False
            i += 1; continue
        if in_block:
            if ch == "*" and nxt == "/":
                in_block = False; i += 2; continue
            i += 1; continue
        if in_str:
            if escape: escape = False
            elif ch == "\\": escape = True
            elif ch == '"': in_str = False
            i += 1; continue
        if ch == "/" and nxt == "/": in_line = True; i += 2; continue
        if ch == "/" and nxt == "*": in_block = True; i += 2; continue
        if ch == '"': in_str = True; i += 1; continue
        if ch == "{": depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0: return i
        i += 1
    return -1

m = re.search(r"\bpub\s+fn\s+initialize_page_allocator\b", src)
add("initialize_page_allocator exists", m is not None)
body = ""
if m:
    open_pos = src.find("{", m.end())
    close_pos = find_matching_brace(src, open_pos) if open_pos != -1 else -1
    if open_pos != -1 and close_pos != -1:
        body = src[open_pos + 1:close_pos]

physical = body.find('crate::println!("physical page allocator:");')
first_install = body.find('crate::page_alloc::install(page_allocator)', physical)
zone_positions = [p for p in [body.find('zone_present_pages', physical), body.find('zone_free_pages', physical)] if p != -1]
first_zone = min(zone_positions) if zone_positions else -1

add("allocator summary exists", physical != -1)
add("page allocator installed in summary path", first_install != -1)
add("riscv install path is cfg gated", '#[cfg(target_arch = "riscv64")]' in body)
add("verbose zone summary gated away from riscv", '#[cfg(not(target_arch = "riscv64"))]' in body or '#[cfg(target_arch = "loongarch64")]' in body)
add("riscv verbose zone summary not required before install", first_install != -1 and (first_zone == -1 or first_install < first_zone))
add("riscv path uses expected free count after install", 'let total_free_pages = expected_free_pages;' in body)
add("no boot trace residue", 'OSCOMP_RISCV_POST_FINAL_TRACE' not in src and 'P0:enter-page-allocator' not in src and 'B0:release-enter' not in src)

failed = [(name, ok) for name, ok in checks if not ok]
for name, ok in checks:
    print(("PASS" if ok else "FAIL") + f": {name}")
print(f"oscomp-riscv-allocator-install-first-audit: PASS={len(checks)-len(failed)}, FAIL={len(failed)}")
if failed:
    sys.exit(1)
