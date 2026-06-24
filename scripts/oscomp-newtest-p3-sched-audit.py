#!/usr/bin/env python3
"""newtest P3 scheduler ABI audit: sched_getscheduler, sched_getparam, sched_setaffinity, sched_setscheduler."""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
checks = []

def add(ok, name):
    checks.append((ok, name))

user = (root / "kernel/src/user.rs").read_text(encoding="utf-8")
syscall_rs = (root / "kernel/src/syscall.rs").read_text(encoding="utf-8")

# Syscall numbers defined
add("SCHED_GETSCHEDULER" in syscall_rs and "SCHED_GETPARAM" in syscall_rs,
    "syscall.rs defines SCHED_GETSCHEDULER and SCHED_GETPARAM")
add("120" in syscall_rs.split("SCHED_GETSCHEDULER")[1][:30] if "SCHED_GETSCHEDULER" in syscall_rs else False,
    "SCHED_GETSCHEDULER = 120")
add("121" in syscall_rs.split("SCHED_GETPARAM")[1][:30] if "SCHED_GETPARAM" in syscall_rs else False,
    "SCHED_GETPARAM = 121")

# Dispatch cases
add("SYS_SCHED_GETSCHEDULER" in user and "SYS_SCHED_GETPARAM" in user,
    "user.rs has SYS_SCHED_GETSCHEDULER and SYS_SCHED_GETPARAM dispatch")

# Implementations
add("fn sys_sched_getscheduler" in user,
    "sys_sched_getscheduler is implemented")
add("SCHED_OTHER as isize" in user,
    "sched_getscheduler returns SCHED_OTHER")
add("fn sys_sched_getparam" in user,
    "sys_sched_getparam is implemented")
add("fn sys_sched_setaffinity" in user and "copy_from_user" in user.split("fn sys_sched_setaffinity")[1].split("fn sys_")[0] if "fn sys_sched_setaffinity" in user else False,
    "sched_setaffinity validates mask via copy_from_user")
add("fn sys_sched_setscheduler" in user,
    "sys_sched_setscheduler is implemented")
add("SCHED_OTHER" in user.split("fn sys_sched_setscheduler")[1].split("fn sys_")[0] if "fn sys_sched_setscheduler" in user else False,
    "sched_setscheduler accepts SCHED_OTHER")

failed = [name for ok, name in checks if not ok]
if failed:
    print("newtest P3 scheduler ABI audit: FAIL")
    for name in failed:
        print("  FAIL:", name)
    sys.exit(1)
print("newtest P3 scheduler ABI audit: PASS")
for _, name in checks:
    print("  PASS:", name)
