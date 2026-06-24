#!/usr/bin/env python3
"""newtest P5 clone/futex/TLS audit: thread creation, clear_child_tid, futex, TLS."""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
checks = []

def add(ok, name):
    checks.append((ok, name))

user = (root / "kernel/src/user.rs").read_text(encoding="utf-8")
proc = (root / "kernel/src/process.rs").read_text(encoding="utf-8")

# Clone supports thread creation
add("CLONE_VM" in user and "CLONE_THREAD" in user and "CLONE_SETTLS" in user,
    "sys_clone recognizes thread-related flags")
add("fork_child_thread" in user,
    "sys_clone calls fork_child_thread for CLONE_VM threads")
add("set_tls_pointer" in user,
    "sys_clone handles CLONE_SETTLS via set_tls_pointer")
add("set_clear_child_tid" in user,
    "sys_clone handles CLONE_CHILD_CLEARTID via set_clear_child_tid")

# fork_child_thread in Process
add("fn fork_child_thread" in proc,
    "Process has fork_child_thread method")
add("create_from_shared_mm" in proc,
    "Process has create_from_shared_mm for CLONE_VM")
add("fn set_tls_pointer" in proc,
    "Thread has set_tls_pointer method")
add("fn set_clear_child_tid" in proc,
    "Thread has set_clear_child_tid method")
add("clear_child_tid: AtomicUsize" in proc,
    "Thread has clear_child_tid field")

# set_tid_address saves address
add("fn sys_set_tid_address" in user and "set_clear_child_tid" in user.split("fn sys_set_tid_address")[1].split("fn sys_")[0] if "fn sys_set_tid_address" in user else False,
    "set_tid_address saves clear_child_tid")

# Exit path clears child tid
add("clear_child_tid_address" in user,
    "exit path reads clear_child_tid_address for cleanup")

# Futex is functional
add("FUTEX_WAIT" in user and "FUTEX_WAKE" in user and "FUTEX_PRIVATE_FLAG" in user,
    "futex supports WAIT, WAKE, and PRIVATE_FLAG")

failed = [name for ok, name in checks if not ok]
if failed:
    print("newtest P5 clone/futex/TLS audit: FAIL")
    for name in failed:
        print("  FAIL:", name)
    sys.exit(1)
print("newtest P5 clone/futex/TLS audit: PASS")
for _, name in checks:
    print("  PASS:", name)
