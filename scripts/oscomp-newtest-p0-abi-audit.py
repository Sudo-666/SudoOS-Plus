#!/usr/bin/env python3
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
checks = []

def add(ok, name):
    checks.append((ok, name))

user = (root / "kernel/src/user.rs").read_text(encoding="utf-8")
exec_rs = (root / "kernel/src/exec.rs").read_text(encoding="utf-8")
lockdep = (root / "kernel/src/lockdep.rs").read_text(encoding="utf-8")

add("SUDOOS_NEWTEST_P0_ABI_HOTFIX_V2" in user, "user.rs marker")
add('b"6.12.0"' in user and 'b"5.4.0"' not in user, "uname reports modern kernel release")
add("SUDOOS_NEWTEST_P0_ABI_HOTFIX_V2" in exec_rs, "exec.rs marker")
for token in ["AT_UID", "AT_EUID", "AT_GID", "AT_EGID", "AT_CLKTCK", "AT_PLATFORM", "AT_HWCAP", "AT_HWCAP2"]:
    add(token in exec_rs, f"auxv has {token}")
add("platform_ptr" in exec_rs, "AT_PLATFORM string is placed on stack")
add("SUDOOS_NEWTEST_P0_ABI_HOTFIX_V2" in lockdep, "lockdep marker")
add("class.rank == LockRank::Console" in lockdep and "held.class.rank == LockRank::Console" in lockdep, "console lockdep order neutralized")

failed = [name for ok, name in checks if not ok]
if failed:
    print("newtest P0 ABI audit: FAIL")
    for name in failed:
        print("  FAIL:", name)
    sys.exit(1)
print("newtest P0 ABI audit: PASS")
for _, name in checks:
    print("  PASS:", name)
