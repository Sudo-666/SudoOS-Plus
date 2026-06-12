#!/usr/bin/env python3
"""Build SudoOS, boot it in QEMU, and validate serial smoke evidence."""

from __future__ import annotations

import argparse
import os
import selectors
import shlex
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT_DIR = Path(__file__).resolve().parent.parent
SUCCESS_MARKER = b"SMOKE_TEST: PASS"
FAILURE_MARKERS = (
    b"SMOKE_TEST: FAIL",
    b"panicked at",
    b"kernel panic",
    b"KERNEL PANIC",
    b"unhandled interrupt",
    b"unexpected RISC-V exception",
    b"unexpected LoongArch exception",
)
COMMON_REQUIRED_MARKERS = (
    b"repeated entry   : verified (3 traps)",
    b"frame guard      : verified",
    b"register restore : verified",
    b"runtime pgtbl  : active hardware root",
    b"hardware access : verified",
    b"table reclaim   : verified",
    b"timer interrupt  : verified",
    b"acknowledge      : verified",
    b"rearm            : verified",
    b"idle wakeup      : verified",
    b"local interrupts : enabled",
    b"periodic timer   : armed at 100 Hz",
    b"kernel threads  : verified (",
    b"private stacks  : verified",
    b"context switch  : verified",
    b"cooperative     : verified",
    b"timer coexistence: verified",
    b"task exit       : verified",
    b"resource reclaim: verified",
    b"non-yielding task : preempted",
    b"timer reschedule  : verified",
    b"preempt count     : verified",
    b"block current     : verified",
    b"no lost wakeup    : verified",
    b"wake one/all      : verified",
    b"M4C_SCHED_TEST: PASS",
    b"SMP_TEST: PASS",
)
ARCH_REQUIRED_MARKERS = {
    "riscv64": (
        b"direct map      : verified",
        b"low boot mapping: removed",
    ),
    "loongarch64": (
        b"refill entry   :",
        b"address bits   : VA=48 PA=48",
    ),
}
MAX_SCAN_BUFFER = 64 * 1024


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--arch", choices=("riscv64", "loongarch64"), required=True)
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--timeout", type=float, default=30.0)
    return parser.parse_args()


def qemu_command(arch: str, kernel: Path) -> list[str]:
    memory = os.environ.get("MEM", "256M")
    cpus = os.environ.get("SMP", "1")

    if arch == "riscv64":
        qemu = os.environ.get("QEMU_RISCV64", "qemu-system-riscv64")
        machine = ["-machine", "virt", "-bios", "default"]
    else:
        qemu = os.environ.get("QEMU_LOONGARCH64", "qemu-system-loongarch64")
        machine = ["-machine", "virt"]

    if shutil.which(qemu) is None:
        raise RuntimeError(f"QEMU executable was not found: {qemu}")

    command = [
        qemu,
        *machine,
        "-m",
        memory,
        "-smp",
        cpus,
        "-display",
        "none",
        "-serial",
        "stdio",
        "-monitor",
        "none",
        "-no-reboot",
        "-kernel",
        str(kernel),
    ]

    command.extend(shlex.split(os.environ.get("QEMU_ARGS", "")))
    return command


def stop_process(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return

    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=2.0)
        return
    except ProcessLookupError:
        return
    except subprocess.TimeoutExpired:
        pass

    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            return

        try:
            process.wait(timeout=2.0)
        except subprocess.TimeoutExpired:
            pass


def main() -> int:
    args = parse_args()

    if args.timeout <= 0:
        raise RuntimeError("--timeout must be greater than zero")

    build_env = os.environ.copy()
    build_env["ARCH"] = args.arch
    build_env["PROFILE"] = args.profile

    subprocess.run(
        [str(ROOT_DIR / "scripts" / "build.sh")],
        cwd=ROOT_DIR,
        env=build_env,
        check=True,
    )

    path_file = ROOT_DIR / "build" / args.arch / "kernel.path"
    kernel = Path(path_file.read_text(encoding="utf-8").strip())

    if not kernel.is_file():
        raise RuntimeError(f"kernel ELF does not exist: {kernel}")

    command = qemu_command(args.arch, kernel)
    log_path = ROOT_DIR / "build" / args.arch / "smoke.log"

    print("smoke command:", shlex.join(command), flush=True)
    print("smoke log:", log_path, flush=True)

    process = subprocess.Popen(
        command,
        cwd=ROOT_DIR,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        bufsize=0,
        start_new_session=True,
    )

    assert process.stdout is not None

    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + args.timeout
    scan_buffer = b""
    success = False
    failure_reason: str | None = None
    cpu_count = int(os.environ.get("SMP", "1"))
    if cpu_count <= 0:
        raise RuntimeError("SMP must be greater than zero")

    smp_markers = (
        f"discovered CPUs : {cpu_count}".encode(),
        f"online CPUs     : {cpu_count}".encode(),
        f"participating CPUs : {cpu_count}".encode(),
        b"per-CPU stacks  : verified",
        b"per-CPU traps   : verified",
        b"per-CPU timers  : armed",
        b"per-CPU current     : verified",
        b"task affinity       : verified",
        b"idle fallback       : verified",
    )
    if cpu_count > 1:
        smp_markers += (
            b"secondary CPUs  : verified",
            b"concurrent threads : verified",
            b"remote wakeup       : verified",
            b"IPI delivery        : verified",
            b"work stealing       : verified (unstarted tasks)",
        )
    else:
        smp_markers += (
            b"secondary CPUs  : single-CPU fallback",
            b"remote wakeup       : single-CPU fallback",
            b"IPI delivery        : single-CPU fallback",
            b"work stealing       : single-CPU fallback",
        )

    required_markers = set(
        COMMON_REQUIRED_MARKERS + ARCH_REQUIRED_MARKERS[args.arch] + smp_markers
    )

    try:
        with log_path.open("wb") as log_file:
            while time.monotonic() < deadline:
                events = selector.select(timeout=0.1)

                for key, _ in events:
                    chunk = os.read(key.fileobj.fileno(), 4096)

                    if not chunk:
                        continue

                    log_file.write(chunk)
                    log_file.flush()

                    sys.stdout.write(chunk.decode("utf-8", errors="replace"))
                    sys.stdout.flush()

                    scan_buffer = (scan_buffer + chunk)[-MAX_SCAN_BUFFER:]
                    required_markers = {
                        marker for marker in required_markers if marker not in scan_buffer
                    }

                    if SUCCESS_MARKER in scan_buffer:
                        if required_markers:
                            missing = ", ".join(
                                repr(marker.decode("ascii", errors="replace"))
                                for marker in sorted(required_markers)
                            )
                            failure_reason = (
                                "success marker arrived before required evidence: " + missing
                            )
                        else:
                            success = True
                        break

                    matched = next(
                        (marker for marker in FAILURE_MARKERS if marker in scan_buffer),
                        None,
                    )
                    if matched is not None:
                        marker_text = matched.decode("ascii", errors="replace")
                        failure_reason = (
                            "serial output contained failure marker: " f"{marker_text}"
                        )
                        break

                if success or failure_reason is not None:
                    break

                return_code = process.poll()
                if return_code is not None:
                    failure_reason = (
                        f"QEMU exited before success marker (status {return_code})"
                    )
                    break

            if not success and failure_reason is None:
                marker_text = SUCCESS_MARKER.decode("ascii")
                failure_reason = (
                    f"timeout after {args.timeout:.1f}s waiting for {marker_text!r}"
                )
    finally:
        selector.close()
        stop_process(process)

    if success:
        print(f"smoke {args.arch}: PASS")
        return 0

    print(f"smoke {args.arch}: FAIL: {failure_reason}", file=sys.stderr)
    print(f"serial log: {log_path}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"smoke test failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
