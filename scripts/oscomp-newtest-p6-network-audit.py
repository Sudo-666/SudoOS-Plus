#!/usr/bin/env python3
"""newtest P6 network audit: sockets, poll, setsockopt/getsockopt, FIONBIO."""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
checks = []

def add(ok, name):
    checks.append((ok, name))

user = (root / "kernel/src/user.rs").read_text(encoding="utf-8")
socket_rs = (root / "kernel/src/net/socket.rs").read_text(encoding="utf-8")

# Socket syscalls dispatched
add("SYS_SETSOCKOPT" in user and "SYS_GETSOCKOPT" in user,
    "user.rs dispatches SETSOCKOPT and GETSOCKOPT")

# setsockopt / getsockopt implemented
add("fn sys_setsockopt" in user,
    "sys_setsockopt is implemented")
add("fn sys_getsockopt" in user,
    "sys_getsockopt is implemented")
add("SO_REUSEADDR" in user and "SO_ERROR" in user,
    "setsockopt/getsockopt handle common socket options")

# FIONBIO ioctl for sockets
add("FIONBIO" in socket_rs,
    "SocketFile ioctl handles FIONBIO")
add("fn ioctl" in socket_rs.split("impl FileOperations for SocketFile")[1].split("impl ")[0] if "impl FileOperations for SocketFile" in socket_rs else False,
    "SocketFile implements ioctl with FIONBIO support")

# Core socket syscalls exist
for name in ["sys_socket", "sys_bind", "sys_listen", "sys_accept",
              "sys_connect", "sys_sendto", "sys_recvfrom", "sys_shutdown"]:
    add(f"pub fn {name}" in socket_rs, f"socket.{name} exists")

# Poll support
add("fn poll" in socket_rs.split("impl FileOperations for SocketFile")[1].split("impl ")[0] if "impl FileOperations for SocketFile" in socket_rs else False,
    "SocketFile implements poll for readiness")

failed = [name for ok, name in checks if not ok]
if failed:
    print("newtest P6 network audit: FAIL")
    for name in failed:
        print("  FAIL:", name)
    sys.exit(1)
print("newtest P6 network audit: PASS")
for _, name in checks:
    print("  PASS:", name)
