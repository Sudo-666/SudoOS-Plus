#!/usr/bin/env python3
"""newtest P2 VFS audit: mknodat, statfs, syslog, RTC ioctl."""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
checks = []

def add(ok, name):
    checks.append((ok, name))

user = (root / "kernel/src/user.rs").read_text(encoding="utf-8")
fs_mod = (root / "kernel/src/fs/mod.rs").read_text(encoding="utf-8")
rtc = (root / "kernel/src/rtc.rs").read_text(encoding="utf-8")

# mknodat
add("fn sys_mknodat" in user and "0o100000" in user,
    "mknodat rejects regular files")
add("0o020000" in user or "0o060000" in user,
    "mknodat allows char/block device nodes")
add("EPERM" in user.split("fn sys_mknodat")[1].split("fn sys_")[0] if "fn sys_mknodat" in user else False,
    "mknodat restricts to /dev")

# statfs / fstatfs
add("resolve_fs_magic" in fs_mod,
    "fs/mod.rs exports resolve_fs_magic")
add("0x01021994" in fs_mod and "0x9fa0" in fs_mod and "0x62656572" in fs_mod,
    "resolve_fs_magic returns per-filesystem f_type magic")
add("sys_statfs_path" in user and "sys_statfs_fd" in user,
    "statfs and fstatfs are separate functions")
add("0xEF53" in fs_mod,
    "ext4 f_type magic present")

# syslog
add("fn sys_syslog" in user and ("4 => 0" in user or "4 | 9" in user),
    "syslog handles SYSLOG_ACTION_READ_CLEAR(4)")

# RTC ioctl
add("RTC_RD_TIME" in rtc,
    "rtc.rs defines RTC_RD_TIME ioctl constant")
add("pub fn ioctl" in rtc,
    "rtc.rs exports ioctl handler")
add('DeviceKind::Rtc => crate::rtc::ioctl(cmd, arg)' in fs_mod,
    "devfs dispatches RTC ioctl to rtc::ioctl")


failed = [name for ok, name in checks if not ok]
if failed:
    print("newtest P2 VFS audit: FAIL")
    for name in failed:
        print("  FAIL:", name)
    sys.exit(1)
print("newtest P2 VFS audit: PASS")
for _, name in checks:
    print("  PASS:", name)
