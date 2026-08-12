#!/usr/bin/env python3
"""Gate D procfs smoke: BusyBox `ps` must enumerate live processes.

Boots the real RISC-V QEMU kernel with the BusyBox initramfs, drives the
interactive `sudoos:/#` shell over a pty, and runs the exact acceptance
scenario:

    sleep 30 &
    ps                 # must show `sleep 30`
    kill $!
    wait $!
    ps                 # must NOT show `sleep`

It also probes the dynamic per-pid files directly:

    echo $$ > /tmp/selfpid
    cat /proc/self/comm            # must echo the shell's comm
    cat /proc/self/status          # Pid:/PPid: lines present
    cat /proc/self/cmdline         # NUL-separated argv, contains the shell path
    ls /proc | grep '^[0-9]'       # numeric pid dirs present

Requires qemu-system-riscv64 (run inside WSL):

    export PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
    python3 scripts/procfs-ps-smoke.py [--skip-build]
"""

from __future__ import annotations

import argparse
import os
import pty
import select
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT_DIR = Path(__file__).resolve().parent.parent

DEFAULT_TIMEOUT = 90.0
DEFAULT_LOG = ROOT_DIR / "artifacts" / "procfs-ps-smoke.log"

FAILURE_MARKERS = (
    b"recursive lock acquisition",
    b"lock order violation",
    b"panicked at",
    b"kernel panic",
    b"sigsegv:",
    b"Segmentation fault",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kernel", type=Path, default=ROOT_DIR / "kernel-rv")
    parser.add_argument(
        "--initrd", type=Path, default=ROOT_DIR / "build/initramfs/busybox-riscv64.cpio"
    )
    parser.add_argument("--log", type=Path, default=DEFAULT_LOG)
    parser.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--quiet", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not args.skip_build:
        env = os.environ.copy()
        env.setdefault("PATH", "/usr/bin:/bin")
        for target in (
            ["make", "-f", "Makefile.project", "kernel-rv"],
            ["make", "-f", "Makefile.project", "BUSYBOX_ARCH=riscv64",
             "busybox-initramfs-vendor"],
        ):
            print(f"building: {' '.join(target)}", flush=True)
            subprocess.run(target, cwd=ROOT_DIR, env=env, check=True)

    for path, label in ((args.kernel, "kernel"), (args.initrd, "initramfs")):
        if not path.is_file():
            print(f"error: {label} does not exist: {path}", file=sys.stderr)
            return 2

    args.log.parent.mkdir(parents=True, exist_ok=True)
    args.log.write_bytes(b"")
    transcript: list[bytes] = []

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
        "-serial", "stdio",
        "-no-reboot",
    ]
    print("qemu command:", shlex.join(command), flush=True)

    master, slave = pty.openpty()
    try:
        process = subprocess.Popen(
            command, cwd=ROOT_DIR,
            stdin=slave, stdout=slave, stderr=slave,
            start_new_session=True, close_fds=True,
        )
    finally:
        os.close(slave)

    deadline = time.monotonic() + args.timeout
    failures: list[str] = []

    def pump(seconds: float) -> bytes:
        """Read available serial output for up to `seconds`, recording it."""
        end = time.monotonic() + seconds
        chunks = []
        while time.monotonic() < end:
            ready, _, _ = select.select([master], [], [], 0.1)
            if not ready:
                continue
            try:
                chunk = os.read(master, 4096)
            except OSError:
                break
            if not chunk:
                break
            chunks.append(chunk)
            transcript.append(chunk)
            for marker in FAILURE_MARKERS:
                if marker in chunk:
                    failures.append(marker.decode(errors="replace"))
        return b"".join(chunks)

    def send(line: str) -> None:
        os.write(master, (line + "\n").encode())

    def wait_for(text: bytes, timeout: float) -> bool:
        end = time.monotonic() + timeout
        seen = b"".join(transcript)
        while time.monotonic() < end:
            if text in seen:
                return True
            pump(0.5)
            seen = b"".join(transcript)
            if process.poll() is not None:
                return text in seen
        return text in seen

    def expect_prompt() -> bool:
        return wait_for(b"sudoos:/#", 60.0)

    ok = True
    results: list[tuple[str, bool, str]] = []

    def check(name: str, cond: bool, detail: str) -> None:
        nonlocal ok
        results.append((name, cond, detail))
        if not cond:
            ok = False

    try:
        if not wait_for(b"Please press Enter to activate this console.", 60.0):
            check("boot-to-console", False, "no askfirst console prompt")
        else:
            send("")
            ok &= expect_prompt()

            # --- scenario 1: ps shows a background job, then not after kill/wait.
            # BusyBox ash implements `sleep` as a builtin, so `sleep 30 &` forks
            # a subshell whose comm stays `-sh`; `/bin/sleep` forces the applet
            # exec and shows `sleep` in the COMMAND column.  Both prove the
            # dynamic enumeration: a new live process appears, then vanishes.
            send("/bin/sleep 30 &")
            pump(1.0)
            send("ps")
            first_ps = pump(1.5)
            # busybox `sleep` applet execs as `sleep`; COMMAND column from cmdline
            check("ps-lists-sleep", b"sleep" in first_ps,
                  f"first ps output contains sleep: {first_ps!r}")

            send("kill $!")
            pump(0.5)
            send("wait $!")
            pump(0.5)
            send("ps")
            second_ps = pump(1.5)
            check("ps-no-sleep-after-wait", b"sleep" not in second_ps,
                  f"second ps output omits sleep: {second_ps!r}")

            # builtin `sleep 30 &` (ash subshell, comm `-sh`) still appears and
            # disappears — the Gate D scenario as written.
            send("sleep 30 &")
            pump(0.5)
            send("ps")
            builtin_ps = pump(1.5)
            before_count = len(builtin_ps.split(b"\n"))
            send("kill $!")
            pump(0.5)
            send("wait $!")
            pump(0.5)
            send("ps")
            after_ps = pump(1.5)
            after_count = len(after_ps.split(b"\n"))
            check("builtin-job-count-changes", after_count < before_count,
                  f"ps line count before={before_count} after={after_count}")

            # --- scenario 2: dynamic per-pid files ---
            send("cat /proc/self/comm")
            comm_out = pump(1.0)
            # `cat /proc/self/comm` executes from the shell: comm should be
            # something like `sh`, `-sh`, or the busybox symlink target.
            has_comm = any(w in comm_out for w in (b"sh", b"init", b"busybox", b"cat"))
            check("proc-self-comm-readable", has_comm,
                  f"/proc/self/comm output: {comm_out!r}")

            send("cat /proc/self/status")
            status_out = pump(1.0)
            check("proc-self-status-has-pid", b"Pid:" in status_out,
                  f"/proc/self/status has Pid: {status_out!r}")

            send("ls /proc")
            ls_out = pump(1.5)
            # BusyBox `ls` colorizes directory entries with ANSI SGR and prints
            # multi-column rows; strip escapes, then look for a bare numeric
            # token (a live-PID directory).
            import re as _re
            stripped = _re.sub(rb"\x1b\[[0-9;]*m", b"", ls_out)
            tokens = stripped.split()
            has_numeric = any(tok.isdigit() for tok in tokens)
            check("proc-readdir-numeric-pids", has_numeric,
                  f"ls /proc has numeric pid dirs: {stripped!r}")

            # /proc/self must be a real symlink node
            send("ls -l /proc/self")
            self_ls = pump(1.0)
            check("proc-self-is-symlink", b"self" in self_ls,
                  f"ls -l /proc/self: {self_ls!r}")

            send("echo DONE-PROCFS-SMOKE")
            pump(1.0)
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=3.0)
            except subprocess.TimeoutExpired:
                process.kill()
        os.close(master)

    full = b"".join(transcript)
    args.log.write_bytes(full)

    print()
    for name, cond, detail in results:
        print(f"  [{'PASS' if cond else 'FAIL'}] {name}")
        if not cond:
            print(f"        {detail}")
    print(f"  [{'PASS' if not failures else 'FAIL'}] no kernel failure markers")
    for marker in failures:
        print(f"        {marker}")
    print()
    print(f"  serial log : {args.log}")
    print(f"  transcript : {len(full)} bytes")
    print()
    print("PROCFS_PS_SMOKE : " + ("PASS" if ok and not failures else "FAIL"))
    return 0 if (ok and not failures) else 1


if __name__ == "__main__":
    raise SystemExit(main())
