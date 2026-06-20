#!/usr/bin/env python3
# SUDOOS_M16A_ELF_AUXV_PATCH_V1
from __future__ import annotations
import sys
from pathlib import Path

checks = []

def read(path: str) -> str:
    try:
        return Path(path).read_text(encoding="utf-8")
    except FileNotFoundError:
        return ""

def check(name: str, ok: bool, detail: str) -> None:
    checks.append((name, ok, detail))

elf = read("kernel/src/elf.rs")
exec_rs = read("kernel/src/exec.rs")

for token in ["ET_DYN", "PT_INTERP", "PT_PHDR", "PT_DYNAMIC", "load_bias", "ProgramHeaderInfo", "DynamicInfo"]:
    check(f"elf metadata {token}", token in elf, f"kernel/src/elf.rs should expose {token}")

for token in ["AT_PHDR", "AT_PHENT", "AT_PHNUM", "AT_BASE", "AT_RANDOM", "AT_EXECFN", "envp"]:
    check(f"exec stack {token}", token in exec_rs, f"kernel/src/exec.rs should build Linux-like stack surface for {token}")

check(
    "dynamic fail-closed",
    "DynamicInterpreterUnsupported" in exec_rs and "elf.interpreter.is_some() || elf.dynamic.is_some()" in exec_rs,
    "M16-A must not jump into dynamic binaries before PT_INTERP/relocations/TLS exist",
)

failures = 0
for name, ok, detail in checks:
    status = "PASS" if ok else "FAIL"
    print(f"[{status}] {name}: {detail}")
    if not ok:
        failures += 1

print(f"\nM16-A ELF/auxv audit: PASS={len(checks)-failures} FAIL={failures}")
raise SystemExit(1 if failures else 0)
