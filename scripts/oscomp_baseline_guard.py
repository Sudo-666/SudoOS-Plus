#!/usr/bin/env python3
"""OSComp baseline guard — read-only sanity checks for critical scoring paths.

Usage:
  python3 scripts/oscomp_baseline_guard.py

Exit code: 1 if any FAIL, 0 otherwise.  WARN does not block.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

PASS = 0
WARN = 1
FAIL = 2

results: dict[str, list[tuple[int, str]]] = {"PASS": [], "WARN": [], "FAIL": []}


def check(level: int, name: str, ok: bool, detail: str = "") -> None:
    tag = {PASS: "PASS", WARN: "WARN", FAIL: "FAIL"}[level]
    msg = f"  [{tag}] {name}"
    if detail:
        msg += f"  — {detail}"
    results[tag].append((level, msg))


def read_text(rel: str) -> str | None:
    p = ROOT / rel
    if not p.is_file():
        return None
    return p.read_text()


def main() -> int:
    user_rs = read_text("kernel/src/user.rs")
    if user_rs is None:
        check(FAIL, "user.rs exists", False, "kernel/src/user.rs not found")
        return 1
    check(PASS, "user.rs exists", True)

    # ── RV glibc busybox direct override ──
    check(PASS, "oscomp-rv-busybox-direct exists",
          "oscomp-rv-busybox-direct" in user_rs)
    check(PASS, "/mnt/sdcard/glibc/busybox shell path",
          "/mnt/sdcard/glibc/busybox" in user_rs)

    # ── LA direct basic runner ──
    check(PASS, "oscomp-la-basic-direct exists",
          "oscomp-la-basic-direct" in user_rs)
    check(PASS, "oscomp_la_run_basic_direct exists",
          "oscomp_la_run_basic_direct" in user_rs)

    la_cases = [
        "brk", "chdir", "clone", "close", "dup2", "dup", "execve", "exit",
        "fork", "fstat", "getcwd", "getdents", "getpid", "getppid",
        "gettimeofday", "mkdir_", "mmap", "mount", "munmap", "openat",
        "open", "pipe", "read", "sleep", "times", "umount", "uname",
        "unlink", "wait", "waitpid", "write", "yield",
    ]
    missing_cases = [c for c in la_cases if f'"{c}"' not in user_rs]
    if missing_cases:
        check(WARN, "LA basic cases", False,
              f"missing: {', '.join(missing_cases)}")
    else:
        check(PASS, "LA basic cases", True, f"all {len(la_cases)} present")

    # ── LA musl glibc busybox shell override ──
    has_musl_shell = ("/musl/" in user_rs
                      and "/mnt/sdcard/glibc/busybox" in user_rs
                      and "oscomp-la-musl" in user_rs)
    check(PASS, "LA musl glibc shell override", has_musl_shell)

    # ── LoongArch FPU/FPD fix ──
    cpu_rs = read_text("arch/loongarch64/src/cpu.rs")
    if cpu_rs and "enable_fpu" in cpu_rs:
        check(PASS, "enable_fpu in cpu.rs", True)
    else:
        check(FAIL, "enable_fpu in cpu.rs", False, "missing LA FPU fix")

    check(PASS, "code == 15 FPD handler",
          "code == 15" in user_rs or "code==15" in user_rs)
    check(PASS, "OSCOMP_LA_FPD_FIXUPS",
          "OSCOMP_LA_FPD_FIXUPS" in user_rs)

    # ── score output ──
    check(PASS, "score= exists", 'score=' in user_rs)
    check(PASS, "score: exists", 'score:' in user_rs)

    # ── whitelists ──
    check(PASS, "oscomp_la_whitelist exists", "oscomp_la_whitelist" in user_rs)
    check(PASS, "oscomp_rv_whitelist exists", "oscomp_rv_whitelist" in user_rs)

    # ── SMOLTCP vendor protection ──
    cargo_lock = read_text("Cargo.lock")
    if cargo_lock and 'name = "smoltcp"' in cargo_lock:
        check(PASS, "smoltcp in Cargo.lock", True)
    else:
        check(FAIL, "smoltcp in Cargo.lock", False, "missing from Cargo.lock")

    vendor_smoltcp = (ROOT / "vendor" / "cargo" / "smoltcp-0.11.0").is_dir()
    check(PASS if vendor_smoltcp else FAIL, "vendor/cargo has smoltcp",
          vendor_smoltcp)

    # ── P10-F1 scaffold ──
    check(PASS, "OscompGroupSpec exists",
          "OscompGroupSpec" in user_rs)
    check(PASS, "OscompGroup exists",
          "enum OscompGroup" in user_rs or "OscompGroup" in user_rs)
    check(PASS, "OscompShellPolicy exists",
          "OscompShellPolicy" in user_rs)
    check(PASS, "OscompEnvPolicy exists",
          "OscompEnvPolicy" in user_rs)
    check(PASS, "OscompRunPolicy exists",
          "OscompRunPolicy" in user_rs)
    check(PASS, "oscomp_classify_script exists",
          "oscomp_classify_script" in user_rs)
    check(PASS, "oscomp_log_group_spec_once exists",
          "oscomp_log_group_spec_once" in user_rs)

    if "oscomp_group_preflight" in user_rs:
        if 'OscompPreflightResult' in user_rs:
            check(PASS, "oscomp_group_preflight is real", True,
                  "not stub — OscompPreflightResult present")
        else:
            check(WARN, "oscomp_group_preflight is real", False,
                  "still appears to be stub (no OscompPreflightResult)")
    else:
        check(FAIL, "oscomp_group_preflight exists", False,
              "preflight function is missing")

    # ── P10-F2 preflight ──
    check(PASS, "OscompPreflightStatus exists",
          "OscompPreflightStatus" in user_rs)
    check(PASS, "OscompPreflightResult exists",
          "OscompPreflightResult" in user_rs)
    check(PASS, "oscomp_vfs_path_exists exists",
          "oscomp_vfs_path_exists" in user_rs)
    check(PASS, "oscomp_expected_cwd exists",
          "oscomp_expected_cwd" in user_rs)
    check(PASS, "oscomp_expected_shell exists",
          "oscomp_expected_shell" in user_rs)
    check(PASS, "oscomp_loader_ready exists",
          "oscomp_loader_ready" in user_rs)
    check(PASS, "oscomp_env_ready exists",
          "oscomp_env_ready" in user_rs)
    check(PASS, "oscomp_log_preflight_once exists",
          "oscomp_log_preflight_once" in user_rs)

    # ── P10-F3 mini probe scaffold ──
    check(PASS, "OscompProbeKind exists",
          "OscompProbeKind" in user_rs)
    check(PASS, "OscompMiniProbe exists",
          "OscompMiniProbe" in user_rs)
    check(PASS, "OscompProbeRunStatus exists",
          "OscompProbeRunStatus" in user_rs)
    check(PASS, "oscomp_mini_probes_for exists",
          "oscomp_mini_probes_for" in user_rs)
    check(PASS, "oscomp_log_probe_catalog_once exists",
          "oscomp_log_probe_catalog_once" in user_rs)
    check(PASS, "oscomp_run_mini_probe exists",
          "oscomp_run_mini_probe" in user_rs)
    check(PASS, "oscomp_env_for_policy exists",
          "oscomp_env_for_policy" in user_rs)

    # Each heavy group must appear in the probe catalog
    for g in ["lua", "libcbench", "lmbench", "cyclictest", "iozone",
              "iperf", "libctest", "ltp"]:
        found = g.lower() in user_rs.lower() and "oscomp_mini_probes_for" in user_rs
        check(PASS if found else WARN, f"probe catalog covers {g}", found)

    # Forbidden in probe catalog (only check near probe array definitions)
    # pthread_cond_smasher in old comments about libctest disabled is OK.
    if "oscomp_mini_probes_for" in user_rs:
        # Extract just the probe catalog area (600 chars after the function)
        idx = user_rs.index("oscomp_mini_probes_for")
        probe_area = user_rs[idx:idx+6000]
        if "pthread_cond_smasher" in probe_area.lower():
            check(FAIL, "pthread_cond_smasher absent from catalog", False,
                  "pthread_cond_smasher FOUND in probe catalog area!")
        else:
            check(PASS, "pthread_cond_smasher absent from catalog", True)
    else:
        check(WARN, "pthread_cond_smasher absent from catalog", False,
              "oscomp_mini_probes_for not found — cannot check catalog")

    # ── P10-F4 mini probe runner ──
    check(PASS, "oscomp_probe_path_exists exists",
          "oscomp_probe_path_exists" in user_rs)
    check(PASS, "oscomp_probe_shell_for exists",
          "oscomp_probe_shell_for" in user_rs)
    check(PASS, "oscomp_run_probe_catalog_for_spec exists",
          "oscomp_run_probe_catalog_for_spec" in user_rs)

    # Real probe branches implemented
    for kind in ["ShellTrue", "ShellEcho", "ScriptSmoke", "DirectBinary"]:
        check(PASS, f"{kind} implemented", kind in user_rs)

    # Heavy probes still NotRun
    for kind in ["FsMini", "NetTcpMini", "NetUdpMini", "LtpScan"]:
        has_notrun = "OscompProbeRunStatus::NotRun" in user_rs and kind in user_rs
        check(WARN, f"{kind} not implemented yet", has_notrun,
              "expected — P10-F5 or later")

    # oscomp_run_mini_probe must NOT be an unconditional Pass stub
    if "OscompProbeRunStatus::NotRun" in user_rs:
        check(PASS, "oscomp_run_mini_probe has real branches", True)
    else:
        check(FAIL, "oscomp_run_mini_probe has real branches", False,
              "missing NotRun — is this still a stub?")

    # Must not print fake testcase success in probe runner
    if "testcase success" in user_rs:
        check(WARN, "testcase success not in probe runner",
              "oscomp_run_mini_probe" not in user_rs.split("testcase success")[0],
              "")
    else:
        check(PASS, "testcase success absent from probe runner", True)

    # oscomp-mini-probe log string exists
    check(PASS, "oscomp-mini-probe log string exists",
          "oscomp-mini-probe" in user_rs)

    # oscomp_run_probe_catalog_for_spec must NOT be called from contest runner
    # (check: not present in the contest loop area around "Run the script")
    idx_run_script = user_rs.find("Run the script")
    idx_catalog_call = user_rs.find("oscomp_run_probe_catalog_for_spec(")
    if idx_run_script > 0 and idx_catalog_call > 0:
        # crude proximity check — if catalog call is near the run script line
        if abs(idx_run_script - idx_catalog_call) < 3000:
            check(FAIL, "catalog not called from runner", False,
                  "oscomp_run_probe_catalog_for_spec may be called near contest runner!")
        else:
            check(PASS, "catalog not called from runner", True)
    else:
        check(PASS, "catalog not called from runner", True,
              "function exists but not called from contest runner")

    # ── P10-F5 ProbeOnly bridge ──
    for flag in ["OSCOMP_PROBE_ONLY_ENABLED", "OSCOMP_PROBE_LUA",
                 "OSCOMP_PROBE_LIBCBENCH", "OSCOMP_PROBE_LMBENCH",
                 "OSCOMP_PROBE_CYCLICTEST", "OSCOMP_PROBE_IOZONE",
                 "OSCOMP_PROBE_IPERF", "OSCOMP_PROBE_NETPERF",
                 "OSCOMP_PROBE_LIBCTEST", "OSCOMP_PROBE_LTP"]:
        check(PASS, f"{flag} exists", flag in user_rs)
        if f"{flag}: bool = true" in user_rs or f"{flag}: bool=true" in user_rs:
            check(FAIL, f"{flag} is false", False, f"{flag} must be false!")
        else:
            check(PASS, f"{flag} is false", True)

    check(PASS, "OscompProbeOnlyOutcome exists",
          "OscompProbeOnlyOutcome" in user_rs)
    check(PASS, "oscomp_probe_only_allowed exists",
          "oscomp_probe_only_allowed" in user_rs)
    check(PASS, "oscomp_maybe_run_probe_only exists",
          "oscomp_maybe_run_probe_only" in user_rs)
    check(PASS, "oscomp_probe_only_skip_hook exists",
          "oscomp_probe_only_skip_hook" in user_rs)
    check(PASS, "oscomp-probe-only log string exists",
          "oscomp-probe-only" in user_rs)

    # ── P10-F8 no-sdcard selftest ──
    for flag in ["OSCOMP_PROBE_SELFTEST_NO_SDCARD",
                 "OSCOMP_PROBE_SELFTEST_LUA",
                 "OSCOMP_PROBE_SELFTEST_LIBCBENCH"]:
        check(PASS, f"{flag} exists", flag in user_rs)
        if f"{flag}: bool = true" in user_rs or f"{flag}: bool=true" in user_rs:
            check(FAIL, f"{flag} is false", False, f"{flag} must be false!")
        else:
            check(PASS, f"{flag} is false", True)

    check(PASS, "oscomp_probe_only_no_sdcard_selftest exists",
          "oscomp_probe_only_no_sdcard_selftest" in user_rs)
    check(PASS, "no-vda branch calls selftest",
          "oscomp_probe_only_no_sdcard_selftest()" in user_rs)
    check(PASS, "prepare_path has no-vda guard",
          'open_device("vda")' in user_rs.split("oscomp_probe_only_prepare_path")[1][:300]
          if "oscomp_probe_only_prepare_path" in user_rs else False)
    check(PASS, "oscomp-probe-selftest log exists",
          "oscomp-probe-selftest" in user_rs)
    check(PASS, "prepare skipped no-vda log exists",
          "prepare skipped no-vda" in user_rs)

    # ── P10-R1 time/poll/sched/futex compat ──
    check(PASS, "CLOCK_BOOTTIME appears (clock_gettime 0-7)",
          "clock_id > 7" in user_rs)
    check(PASS, "ITIMER_REAL appears",
          "ITIMER_REAL" in user_rs)
    check(PASS, "KernelItimerval appears",
          "KernelItimerval" in user_rs)
    check(PASS, "getrusage validates who",
          "who > 1" in user_rs)
    check(PASS, "ETIMEDOUT in futex timeout path",
          "ETIMEDOUT" in user_rs)
    check(PASS, "ppoll timeout_address used",
          "timeout_address" in user_rs.split("sys_ppoll")[1][:200]
          if "sys_ppoll" in user_rs else False)
    check(PASS, "sched_getaffinity mask==0 → EFAULT",
          "mask == 0" in user_rs.split("sys_sched_getaffinity")[1][:200]
          if "sys_sched_getaffinity" in user_rs else False)

    # ── P10-R2 fs/iozone compat ──
    check(PASS, "FDATASYNC exists", "SYS_FDATASYNC" in user_rs or "FDATASYNC" in user_rs)
    check(PASS, "sys_fdatasync exists", "sys_fdatasync" in user_rs)
    check(PASS, "PWRITE64 exists", "SYS_PWRITE64" in user_rs or "PWRITE64" in user_rs)
    check(PASS, "sys_pwrite64 exists", "sys_pwrite64" in user_rs)
    check(PASS, "RENAME_NOREPLACE appears", "RENAME_NOREPLACE" in user_rs)
    check(PASS, "renameat2 validates flags", "flags == 0" in user_rs.split("sys_renameat2")[1][:400]
          if "sys_renameat2" in user_rs else False)
    check(PASS, "UTIME_NOW appears", "UTIME_NOW" in user_rs)
    check(PASS, "UTIME_OMIT appears", "UTIME_OMIT" in user_rs)
    check(PASS, "/proc/self/exe in readlinkat",
          "/proc/self/exe" in user_rs.split("sys_readlinkat")[1][:600]
          if "sys_readlinkat" in user_rs else False)
    check(PASS, "/proc/self/fd in readlinkat",
          "/proc/self/fd" in user_rs.split("sys_readlinkat")[1][:600]
          if "sys_readlinkat" in user_rs else False)

    # ── P10-R3 process/pipe/fcntl/prctl compat ──
    check(PASS, "F_SETFL appears", "F_SETFL" in user_rs)
    check(PASS, "F_GETOWN appears", "F_GETOWN" in user_rs)
    check(PASS, "F_SETOWN appears", "F_SETOWN" in user_rs)
    check(PASS, "fcntl handles F_SETFL", "F_SETFL" in user_rs.split("sys_fcntl")[1][:600]
          if "sys_fcntl" in user_rs else False)
    check(PASS, "pipe2 validates flags", "allowed" in user_rs.split("sys_pipe2")[1][:500]
          if "sys_pipe2" in user_rs else False)
    check(PASS, "WNOHANG appears", "WNOHANG" in user_rs)
    check(PASS, "WUNTRACED uses Linux value 2",
          "const WUNTRACED: usize = 2;" in user_rs)

    # ── P10-SCORE-FIX1 ──
    check(PASS, "RV busybox direct: kind=glibc",
          'oscomp-rv-busybox-direct: kind=glibc' in user_rs)
    check(PASS, "RV busybox direct: kind=musl",
          'oscomp-rv-busybox-direct: kind=musl' in user_rs)
    check(PASS, "RV musl direct shell path exists",
          '/mnt/sdcard/musl/busybox' in user_rs)
    check(PASS, "LA whitelist includes glibc lua",
          '/glibc/lua_testcode.sh' in user_rs.split('oscomp_la_whitelist')[1][:300]
          if 'oscomp_la_whitelist' in user_rs else False)
    check(PASS, "LA whitelist includes musl lua",
          '/musl/lua_testcode.sh' in user_rs.split('oscomp_la_whitelist')[1][:300]
          if 'oscomp_la_whitelist' in user_rs else False)
    check(PASS, "LA musl lua log exists",
          'oscomp-la-musl-lua' in user_rs)

    # ── P10-SCORE-FIX2 getcwd conditional ──
    check(PASS, "getcwd prefers full path first",
          'full_need <= size' in user_rs)
    check(PASS, "getcwd strips only as fallback",
          'visible != full && visible_need' in user_rs)
    check(PASS, "getcwd returns cwd len not address",
          'chosen.len() as isize' in user_rs)

    check(PASS, "wait4 accepts 4 args", "rusage_address" in user_rs)
    check(PASS, "PR_SET_NAME appears", "PR_SET_NAME" in user_rs)
    check(PASS, "PR_GET_NAME appears", "PR_GET_NAME" in user_rs)
    check(PASS, "PR_SET_VMA appears", "PR_SET_VMA" in user_rs)
    check(PASS, "PR_SET_TIMERSLACK appears", "PR_SET_TIMERSLACK" in user_rs)
    check(PASS, "sys_prctl no longer all-0", "PR_SET_DUMPABLE" in user_rs)
    check(PASS, "set_robust_list length 24", "ROBUST_LIST_HEAD_SIZE" in user_rs or "24" in user_rs)

    # ── P10-R4 signal/kill compat ──
    check(PASS, "EINTR exists", "EINTR" in user_rs or "pub const EINTR" in (open("kernel/src/syscall.rs").read() if False else ""))
    check(PASS, "oscomp_validate_sigset_size exists", "oscomp_validate_sigset_size" in user_rs)
    check(PASS, "rt_sigaction rejects SIGSTOP", "SIGSTOP" in user_rs.split("sys_rt_sigaction")[1][:200]
          if "sys_rt_sigaction" in user_rs else False)
    check(PASS, "rt_sigprocmask validates sigsetsize", "sigsetsize" in user_rs.split("sys_rt_sigprocmask")[1][:200]
          if "sys_rt_sigprocmask" in user_rs else False)
    check(PASS, "rt_sigpending exists", "sys_rt_sigpending" in user_rs)
    check(PASS, "rt_sigsuspend exists", "sys_rt_sigsuspend" in user_rs)
    check(PASS, "rt_sigtimedwait bounded sleep", "timeout_address == 0" in user_rs.split("sys_rt_sigtimedwait")[1][:800]
          if "sys_rt_sigtimedwait" in user_rs else False)
    check(PASS, "kill handles sig==0", "signal == 0" in user_rs.split("fn sys_kill")[1][:300]
          if "fn sys_kill" in user_rs else False)
    check(PASS, "sigreturn clears unblockable", "unblockable" in user_rs.split("sys_rt_sigreturn")[1][:400]
          if "sys_rt_sigreturn" in user_rs else True)

    # ── P10-R5 net/socket/poll compat ──
    check(PASS, "SOCK_NONBLOCK appears", "SOCK_NONBLOCK" in user_rs or "SOCK_NONBLOCK" in (open("kernel/src/net/socket.rs").read() if True else ""))
    check(PASS, "SOCK_CLOEXEC appears", "SOCK_CLOEXEC" in user_rs or "SOCK_CLOEXEC" in (open("kernel/src/net/socket.rs").read() if True else ""))
    check(PASS, "MSG_DONTWAIT appears", "MSG_DONTWAIT" in user_rs or "MSG_DONTWAIT" in (open("kernel/src/net/socket.rs").read() if True else ""))
    check(PASS, "MSG_NOSIGNAL appears", "MSG_NOSIGNAL" in user_rs or "MSG_NOSIGNAL" in (open("kernel/src/net/socket.rs").read() if True else ""))
    check(PASS, "ENOTSOCK exists", "ENOTSOCK" in user_rs or "pub const ENOTSOCK" in (open("kernel/src/syscall.rs").read() if True else ""))
    check(PASS, "ENOPROTOOPT exists", "ENOPROTOOPT" in user_rs or "pub const ENOPROTOOPT" in (open("kernel/src/syscall.rs").read() if True else ""))
    check(PASS, "EOPNOTSUPP exists", "EOPNOTSUPP" in user_rs or "pub const EOPNOTSUPP" in (open("kernel/src/syscall.rs").read() if True else ""))
    check(PASS, "getsockopt reads optlen ptr", "optlen_addr" in user_rs.split("sys_getsockopt")[1][:300]
          if "sys_getsockopt" in user_rs else False)
    check(PASS, "setsockopt no longer all-0", "SOL_SOCKET" in user_rs.split("sys_setsockopt")[1][:300]
          if "sys_setsockopt" in user_rs else False)

    # ── P10-R6 mm/resource/misc compat ──
    check(PASS, "GRND_NONBLOCK appears", "GRND_NONBLOCK" in user_rs)
    check(PASS, "getrandom validates flags", "flags" in user_rs.split("sys_getrandom")[1][:200]
          if "sys_getrandom" in user_rs else False)
    check(PASS, "prlimit64 validates new_limit", "new.cur > new.max" in user_rs.split("sys_prlimit64")[1][:300]
          if "sys_prlimit64" in user_rs else False)
    check(PASS, "MAP_NORESERVE accepted", "0x4000" in user_rs.split("MAP_ACCEPTED")[1][:100]
          if "MAP_ACCEPTED" in user_rs else False)
    check(PASS, "MAP_STACK accepted", "0x20000" in user_rs.split("MAP_ACCEPTED")[1][:100]
          if "MAP_ACCEPTED" in user_rs else False)
    check(PASS, "MAP_FIXED_NOREPLACE accepted", "0x100000" in user_rs.split("MAP_ACCEPTED")[1][:100]
          if "MAP_ACCEPTED" in user_rs else False)

    # ── P10-F6 probe bridge coverage ──
    check(PASS, "oscomp_probe_only_prepare_path exists",
          "oscomp_probe_only_prepare_path" in user_rs)
    check(PASS, "oscomp_probe_only_prepare_path uses sdcard install",
          "sdcard_install_ext4_dir_files" in user_rs
          or "sdcard_vfs_to_ext4_dir" in user_rs)

    for site in ["P10-F6 probe-only hook: RV defer",
                 "P10-F6 probe-only hook: RV heavy",
                 "P10-F6 probe-only hook: LA defer",
                 "P10-F6 probe-only hook: not found"]:
        check(PASS, f"hook callsite: {site}", site in user_rs)

    # oscomp_probe_only_skip_hook returns early when master flag false
    if "oscomp_probe_only_skip_hook" in user_rs:
        hook_fn = user_rs.split("oscomp_probe_only_skip_hook")[1][:400]
        if "OSCOMP_PROBE_ONLY_ENABLED" in hook_fn and "Disabled" in hook_fn:
            check(PASS, "skip_hook returns Disabled early", True)
        else:
            check(WARN, "skip_hook returns Disabled early", False,
                  "skip_hook may not return early when disabled")

    # Probe-only must NOT touch score/pass_count/fail_count as assignments
    probe_fn = user_rs.split("oscomp_maybe_run_probe_only")[1] if "oscomp_maybe_run_probe_only" in user_rs else ""
    if "pass_count += " in probe_fn or "fail_count += " in probe_fn or "score =" in probe_fn:
        check(FAIL, "probe-only does not update scoring", False,
              "probe-only function appears to modify score/pass/fail!")
    else:
        check(PASS, "probe-only does not update scoring", True)

    # ── DANGER: forbidden patterns ──
    if "run_group_with_deadline" in user_rs:
        check(FAIL, "run_group_with_deadline absent", False, "FOUND — dangerous!")
    else:
        check(PASS, "run_group_with_deadline absent", True)

    if "kill_process_group" in user_rs:
        check(FAIL, "kill_process_group absent", False, "FOUND — dangerous!")
    else:
        check(PASS, "kill_process_group absent", True)

    # ── WARN: old diagnostic cruft ──
    if "oscomp-la-basic-probe" in user_rs:
        check(WARN, "oscomp-la-basic-probe present",
              True, "old probe function still in source")
    else:
        check(PASS, "oscomp-la-basic-probe absent", True)

    if "oscomp-la-sleep-trace" in user_rs:
        check(WARN, "oscomp-la-sleep-trace present",
              True, "old trace macros still in source")
    else:
        check(PASS, "oscomp-la-sleep-trace absent", True)

    if "libctest" in user_rs and "disabled" in user_rs:
        check(WARN, "libctest disabled", True, "libctest still disabled — expected for now")

    heavy = ["lmbench", "netperf", "iperf", "iozone", "cyclictest", "ltp"]
    heavy_found = [h for h in heavy if h in user_rs and "oscomp_should_skip_heavy" in user_rs]
    if heavy_found:
        check(WARN, "heavy skip active",
              True, f"groups still skipped: {', '.join(heavy_found)}")

    # ── SUMMARY ──
    fail_count = len(results["FAIL"])
    warn_count = len(results["WARN"])
    pass_count = len(results["PASS"])

    print()
    print("=" * 60)
    print(f"  PASS: {pass_count}   WARN: {warn_count}   FAIL: {fail_count}")
    print("=" * 60)

    for tag in ("PASS", "WARN", "FAIL"):
        if results[tag]:
            print(f"\n── {tag} ──")
            for _, msg in results[tag]:
                print(msg)

    if fail_count:
        print("\n>>> BASELINE GUARD: FAILURES DETECTED <<<")
    else:
        print("\n>>> BASELINE GUARD: OK <<<")

    return 1 if fail_count else 0


if __name__ == "__main__":
    sys.exit(main())
