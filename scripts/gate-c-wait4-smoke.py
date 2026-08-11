#!/usr/bin/env python3
"""Gate C 4.8.18: PID 1 interruptible-wait regression through a real RISC-V
QEMU boot of BusyBox /init.

The syscall/process code under test is shared across architectures:

    kernel/src/user.rs
    kernel/src/task/mod.rs
    kernel/src/task/wait_queue.rs
    kernel/src/process.rs

so a RISC-V QEMU run exercises exactly the same `wait4 -> interruptible wait
-> scheduler wake` path that deadlocked on the LS2K1000 board. This harness
boots the real BusyBox `/init` as PID 1 and asserts the full chain:

    INIT: exec pid=1 path=/init                PID 1 is up
    SUDOOS_INIT_READY                          sysinit child ran (echo) and exited
    Please press Enter to activate this console.   PID 1's wait4 was woken,
                                                  reaped the child, and init
                                                  continued to askfirst

Only `Please press Enter ...` proves the parent actually RETURNED from wait4:
it is produced after the sysinit fork/wait cycle completes. The kernel prints
`INIT: exec pid=1` before exec, and the inittab sysinit action echoes the
ready marker from the child.

It also rejects the 4.8.17-class recursive lock:

    SCHEDULER held -> should_block() -> current_user_thread()
                                      -> re-acquire SCHEDULER

via the lockdep failure markers the kernel emits on deadlock.

Run inside WSL (qemu-system-riscv64 lives there):

    export PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
    make gate-c-wait4-smoke

or directly:

    python3 scripts/gate-c-wait4-smoke.py [--skip-build]
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT_DIR = Path(__file__).resolve().parent.parent

# Every one of these must appear in the serial log.
REQUIRED_MARKERS = (
    b"INIT: exec pid=1 path=/init",
    b"SUDOOS_INIT_READY",
    b"Please press Enter to activate this console.",
)

# Any one of these fails the run immediately (kernel deadlock / panic).
FAILURE_MARKERS = (
    b"recursive lock acquisition",
    b"lock order violation",
    b"panicked at",
    b"kernel panic",
    b"KERNEL PANIC",
)

DEFAULT_TIMEOUT = 60.0
DEFAULT_LOG = ROOT_DIR / "artifacts" / "gate-c-wait4-rv.log"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kernel", type=Path, default=ROOT_DIR / "kernel-rv")
    parser.add_argument(
        "--initrd", type=Path, default=ROOT_DIR / "build/initramfs/busybox-riscv64.cpio"
    )
    parser.add_argument("--log", type=Path, default=DEFAULT_LOG)
    parser.add_argument("--result-json", type=Path)
    parser.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--quiet", action="store_true")
    return parser.parse_args()


def git_head() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT_DIR, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def write_json(path: Path, value: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    tmp.replace(path)


def stop_process(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=2.0)
    except (ProcessLookupError, subprocess.TimeoutExpired):
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=2.0)
        except subprocess.TimeoutExpired:
            pass


def main() -> int:
    args = parse_args()
    if not args.skip_build:
        env = os.environ.copy()
        env.setdefault("PATH", "/usr/bin:/bin")
        for target in (
            ["make", "-f", "Makefile.project", "kernel-rv"],
            ["make", "-f", "Makefile.project", "BUSYBOX_ARCH=riscv64", "busybox-initramfs-vendor"],
        ):
            print(f"building: {' '.join(target)}", flush=True)
            subprocess.run(target, cwd=ROOT_DIR, env=env, check=True)

    for path, label in ((args.kernel, "kernel"), (args.initrd, "initramfs")):
        if not path.is_file():
            print(f"error: {label} does not exist: {path}", file=sys.stderr)
            return 2

    args.log.parent.mkdir(parents=True, exist_ok=True)
    result_path = args.result_json or args.log.with_suffix(".json")
    start = time.monotonic()

    result: dict[str, object] = {
        "schema_version": 1,
        "status": "error",
        "reason": "not started",
        "arch": "riscv64",
        "git_head": git_head(),
        "timeout_seconds": args.timeout,
        "serial_log": str(args.log),
        "kernel": str(args.kernel),
        "initrd": str(args.initrd),
    }

    # Truncate the serial log so a stale PASS can never satisfy this run.
    args.log.write_bytes(b"")

    qemu = shutil.which("qemu-system-riscv64") or "qemu-system-riscv64"
    command = [
        qemu,
        "-machine", "virt",
        "-bios", "default",
        "-kernel", str(args.kernel),
        "-initrd", str(args.initrd),
        "-append", "console=ttyS0 rdinit=/init init.debug=1",
        "-m", os.environ.get("MEM", "256M"),
        "-smp", os.environ.get("SMP", "1"),
        "-display", "none",
        "-monitor", "none",
        "-serial", f"file:{args.log}",
        "-no-reboot",
    ]
    result["qemu_command"] = command
    print("qemu command:", shlex.join(command), flush=True)
    print("serial log:", args.log, flush=True)

    # qemu stderr (firmware / machine warnings) goes to a side file, kept out
    # of the serial log so marker analysis stays clean.
    stderr_path = args.log.with_suffix(".qemu-stderr")
    try:
        with stderr_path.open("wb") as stderr_file:
            process = subprocess.Popen(
                command, cwd=ROOT_DIR, stdout=stderr_file, stderr=stderr_file,
                start_new_session=True,
            )
            deadline = time.monotonic() + args.timeout
            seen: set[bytes] = set()
            failure: bytes | None = None
            terminated_by_harness = False

            try:
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        result["qemu_returncode"] = process.returncode
                        break
                    try:
                        data = args.log.read_bytes()
                    except OSError:
                        data = b""
                    if not args.quiet and data:
                        sys.stdout.write(data.decode(errors="replace"))
                        sys.stdout.flush()
                    for marker in REQUIRED_MARKERS:
                        if marker in data:
                            seen.add(marker)
                    for marker in FAILURE_MARKERS:
                        if marker in data:
                            failure = marker
                            break
                    if failure is not None:
                        break
                    if all(m in seen for m in REQUIRED_MARKERS):
                        break
                    time.sleep(0.25)
                else:
                    result["classification"] = "timeout"
                    result["reason"] = f"timeout after {args.timeout:.1f}s"
            finally:
                terminated_by_harness = process.poll() is None
                stop_process(process)

        data = args.log.read_bytes()
        elapsed = time.monotonic() - start
        result["total_seconds"] = round(elapsed, 6)
        result["bytes_received"] = len(data)
        result["last_serial_lines"] = data.decode(errors="replace").splitlines()[-40:]
        result["qemu_stderr"] = stderr_path.read_text(errors="replace")[:4000]
        result["terminated_by_harness"] = terminated_by_harness

        missing = [m.decode(errors="replace") for m in REQUIRED_MARKERS if m not in seen]
        result["required_markers"] = [m.decode(errors="replace") for m in REQUIRED_MARKERS]
        result["seen_markers"] = [m.decode(errors="replace") for m in REQUIRED_MARKERS if m in seen]
        result["missing_markers"] = missing

        if failure is not None:
            result.update({
                "status": "fail",
                "classification": "kernel-failure",
                "reason": f"serial output contained failure marker: {failure.decode(errors='replace')}",
            })
            write_json(result_path, result)
            print(f"gate-c-wait4 smoke: FAIL: {result['reason']}", file=sys.stderr)
            return 1

        if not missing:
            result.update({
                "status": "pass",
                "classification": "pass",
                "reason": "all PID 1 wait4 evidence observed",
            })
            write_json(result_path, result)
            print(
                f"gate-c-wait4 smoke: PASS "
                f"(boot-to-askfirst={elapsed:.3f}s, {len(data)} bytes)",
                flush=True,
            )
            return 0

        result.update({
            "status": "fail",
            "classification": "missing-evidence",
            "reason": f"missing marker(s): {', '.join(missing)}",
        })
        write_json(result_path, result)
        print(f"gate-c-wait4 smoke: FAIL: {result['reason']}", file=sys.stderr)
        print(f"result json: {result_path}", file=sys.stderr)
        return 1

    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        result.update({
            "status": "error",
            "classification": "harness-error",
            "reason": str(error),
            "total_seconds": round(time.monotonic() - start, 6),
        })
        write_json(result_path, result)
        print(f"gate-c-wait4 smoke error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
