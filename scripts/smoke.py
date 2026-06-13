#!/usr/bin/env python3
"""Build SudoOS, boot it in QEMU, and write structured smoke evidence."""

from __future__ import annotations

import argparse
import json
import os
import selectors
import shlex
import shutil
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

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
    b"lock order violation",
    b"recursive lock acquisition",
    b"TLB request timed out",
    b"call-function request timed out",
    b"invalid CPU lifecycle transition",
)

STABLE_COMMON_MARKERS = (
    ("mm", b"runtime pgtbl : active hardware root"),
    ("timer", b"periodic timer : armed at 100 Hz"),
    ("wait", b"M4C_SCHED_TEST: PASS"),
    ("tlb", b"M4C_TLB_TEST: PASS"),
    ("final", b"SMP_TEST: PASS"),
)

# Human-readable detail lines improve diagnostics, but they are not a stable
# serial ABI. Their absence is recorded without overriding a valid kernel PASS.
OPTIONAL_DETAIL_MARKERS = (
    ("trap", b"repeated entry : verified (3 traps)"),
    ("trap", b"frame guard : verified"),
    ("trap", b"register restore : verified"),
    ("mm", b"hardware access : verified"),
    ("mm", b"table reclaim : verified"),
    ("timer", b"timer interrupt : verified"),
    ("timer", b"acknowledge : verified"),
    ("timer", b"rearm : verified"),
    ("timer", b"idle wakeup : verified"),
    ("timer", b"local interrupts : enabled"),
    ("scheduler", b"kernel threads : verified ("),
    ("scheduler", b"private stacks : verified"),
    ("scheduler", b"context switch : verified"),
    ("scheduler", b"cooperative : verified"),
    ("scheduler", b"timer coexistence: verified"),
    ("scheduler", b"task exit : verified"),
    ("scheduler", b"resource reclaim: verified"),
    ("scheduler", b"non-yielding task : preempted"),
    ("scheduler", b"timer reschedule : verified"),
    ("scheduler", b"preempt count : verified"),
    ("wait", b"block current : verified"),
    ("wait", b"no lost wakeup : verified"),
    ("wait", b"switching-out race: verified"),
    ("wait", b"wake one/all : verified"),
    ("tlb", b"local invalidate : verified"),
    ("tlb", b"remap visibility : verified"),
    ("tlb", b"page reclaim : verified"),
    ("tlb", b"affinity release : verified"),
    ("tlb", b"stack/TLB safety : verified"),
)

DEBUG_M5_MARKERS = (
    ("locks", b"tracked spin lock test:"),
    ("wait", b"wait queue/completion invariant test:"),
    ("idle", b"deterministic idle/IPI test:"),
    ("ipi", b"IPI mailbox test:"),
    ("ipi", b"call-function IPI test:"),
    ("tlb", b"TLB request v2 test:"),
)

ARCH_REQUIRED_MARKERS = {
    "riscv64": (("boot", b"direct map : verified"),),
    "loongarch64": (("boot", b"address bits : VA=48 PA=48"),),
}

ARCH_OPTIONAL_MARKERS = {
    "riscv64": (("boot", b"low boot mapping: removed"),),
    "loongarch64": (("boot", b"refill entry :"),),
}

PHASE_ORDER = (
    "boot", "mm", "trap", "timer", "smp", "locks",
    "scheduler", "wait", "idle", "ipi", "tlb", "final",
)
MAX_SCAN_BUFFER = 512 * 1024
TAIL_BYTES = 32 * 1024


@dataclass(frozen=True)
class Marker:
    phase: str
    value: bytes


def normalize_evidence(data: bytes) -> bytes:
    """Normalize presentation whitespace while preserving serial line boundaries.

    Kernel status output aligns labels with runs of spaces. Those runs are
    presentation, not ABI. Each line is independently normalized so evidence
    cannot accidentally match across two different serial lines.
    """
    normalized_lines: list[bytes] = []
    for line in data.replace(b"\r", b"\n").split(b"\n"):
        normalized = b" ".join(line.split())
        if normalized:
            normalized_lines.append(normalized)
    return b"\n".join(normalized_lines)


class MarkerTracker:
    def __init__(self, markers: Iterable[Marker]) -> None:
        self.markers = tuple(markers)
        self.remaining = {m.value: m.phase for m in self.markers}
        self.normalized_values = {
            marker.value: normalize_evidence(marker.value)
            for marker in self.markers
        }
        self.marker_seconds: dict[str, float] = {}
        self.phase_seconds: dict[str, float] = {}
        self.phase_markers: dict[str, set[bytes]] = {}
        self.phase_seen: dict[str, set[bytes]] = {}
        self.last_progress_seconds = 0.0
        self.last_observed_phase = "boot"
        self.last_observed_phase_index = -1
        for marker in self.markers:
            self.phase_markers.setdefault(marker.phase, set()).add(marker.value)
            self.phase_seen.setdefault(marker.phase, set())

    def feed(self, buffer: bytes, elapsed: float) -> list[str]:
        completed = []
        normalized_buffer = normalize_evidence(buffer)
        for value, phase in tuple(self.remaining.items()):
            if self.normalized_values[value] not in normalized_buffer:
                continue
            self.remaining.pop(value)
            self.marker_seconds[value.decode(errors="replace")] = elapsed
            self.phase_seen[phase].add(value)
            self.last_progress_seconds = elapsed

            try:
                phase_index = PHASE_ORDER.index(phase)
            except ValueError:
                phase_index = len(PHASE_ORDER)

            if phase_index >= self.last_observed_phase_index:
                self.last_observed_phase_index = phase_index
                self.last_observed_phase = phase

            if (
                phase not in self.phase_seconds
                and self.phase_seen[phase] == self.phase_markers[phase]
            ):
                self.phase_seconds[phase] = elapsed
                completed.append(phase)
        return completed

    def current_phase(self) -> str:
        """Return the latest phase for which any stable marker was observed.

        This is progress telemetry, not an evidence-completeness calculation.
        One missing early marker must not make a run that reached final appear
        stuck in boot.
        """
        return self.last_observed_phase

    def first_incomplete_phase(self) -> str | None:
        """Return the earliest phase that still lacks required evidence."""
        for phase in PHASE_ORDER:
            if phase not in self.phase_markers:
                continue
            if self.phase_seen[phase] != self.phase_markers[phase]:
                return phase
        return None

    def completed(self) -> list[str]:
        return [p for p in PHASE_ORDER if p in self.phase_seconds]

    def missing(self) -> list[str]:
        return sorted(v.decode(errors="replace") for v in self.remaining)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--arch", choices=("riscv64", "loongarch64"), required=True)
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--timeout", type=float, default=120.0)
    parser.add_argument("--failure-drain-seconds", type=float, default=1.0)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--kernel", type=Path)
    parser.add_argument("--log", type=Path)
    parser.add_argument("--result-json", type=Path)
    parser.add_argument("--quiet", action="store_true")
    return parser.parse_args()


def _deduplicate_markers(
    pairs: Iterable[tuple[str, bytes]],
) -> tuple[Marker, ...]:
    result: list[Marker] = []
    seen: set[bytes] = set()
    for phase, value in pairs:
        if value in seen:
            continue
        seen.add(value)
        result.append(Marker(phase, value))
    return tuple(result)


def required_markers(
    arch: str,
    profile: str,
    cpu_count: int,
) -> tuple[Marker, ...]:
    pairs = list(STABLE_COMMON_MARKERS)
    pairs.extend(ARCH_REQUIRED_MARKERS[arch])
    pairs.extend(
        (
            ("smp", f"discovered CPUs : {cpu_count}".encode()),
            ("smp", f"online CPUs : {cpu_count}".encode()),
            (
                "smp",
                b"secondary CPUs : verified"
                if cpu_count > 1
                else b"secondary CPUs : single-CPU fallback",
            ),
        )
    )
    if cpu_count > 1:
        pairs.extend(
            (
                ("ipi", b"IPI delivery : verified"),
                ("tlb", b"remote invalidate : verified"),
            )
        )
    if profile == "debug":
        pairs.extend(DEBUG_M5_MARKERS)
    return _deduplicate_markers(pairs)


def optional_markers(
    arch: str,
    profile: str,
    cpu_count: int,
) -> tuple[Marker, ...]:
    pairs = list(OPTIONAL_DETAIL_MARKERS)
    pairs.extend(ARCH_OPTIONAL_MARKERS[arch])
    pairs.extend(
        (
            ("smp", b"per-CPU stacks : verified"),
            ("smp", b"per-CPU traps : verified"),
            ("smp", b"per-CPU timers : armed"),
            ("smp", b"per-CPU current : verified"),
            ("smp", b"task affinity : verified"),
            ("smp", b"idle fallback : verified"),
            ("scheduler", b"concurrent threads : verified"),
        )
    )
    if cpu_count > 1:
        pairs.extend(
            (
                ("scheduler", b"remote wakeup : verified"),
                ("scheduler", b"work stealing : verified (runnable task migration)"),
                ("tlb", b"remote ack : verified"),
                ("scheduler", b"started task : migrated CPU0 -> CPU1"),
            )
        )
    else:
        pairs.extend(
            (
                ("scheduler", b"remote wakeup : single-CPU fallback"),
                ("ipi", b"IPI delivery : single-CPU fallback"),
                ("scheduler", b"work stealing : single-CPU fallback"),
                ("tlb", b"remote invalidate : single-CPU fallback"),
                ("tlb", b"remote ack : single-CPU fallback"),
                ("scheduler", b"started task : single-CPU fallback"),
            )
        )
    required_values = {m.value for m in required_markers(arch, profile, cpu_count)}
    return tuple(m for m in _deduplicate_markers(pairs) if m.value not in required_values)

def qemu_command(arch: str, kernel: Path | None) -> list[str]:
    override = os.environ.get("SMOKE_COMMAND")
    if override:
        command = shlex.split(override)
        if not command:
            raise RuntimeError("SMOKE_COMMAND is empty")
        return command
    if kernel is None:
        raise RuntimeError("kernel path is required")
    mem, cpus = os.environ.get("MEM", "256M"), os.environ.get("SMP", "1")
    if arch == "riscv64":
        qemu = os.environ.get("QEMU_RISCV64", "qemu-system-riscv64")
        machine = ["-machine", "virt", "-bios", "default"]
    else:
        qemu = os.environ.get("QEMU_LOONGARCH64", "qemu-system-loongarch64")
        machine = ["-machine", "virt"]
    if shutil.which(qemu) is None:
        raise RuntimeError(f"QEMU executable was not found: {qemu}")
    command = [
        qemu, *machine, "-m", mem, "-smp", cpus, "-display", "none",
        "-serial", "stdio", "-monitor", "none", "-no-reboot",
        "-kernel", str(kernel),
    ]
    command.extend(shlex.split(os.environ.get("QEMU_ARGS", "")))
    return command


def git_head() -> str | None:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT_DIR, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
    )
    return result.stdout.strip() if result.returncode == 0 else None


def write_json(path: Path, value: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    tmp.replace(path)


def stop_process(process: subprocess.Popen[bytes]) -> tuple[bool, int | None]:
    if process.poll() is not None:
        return False, process.returncode
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=2.0)
    except ProcessLookupError:
        pass
    except subprocess.TimeoutExpired:
        if process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=2.0)
            except subprocess.TimeoutExpired:
                pass
    return True, process.poll()


def timeout_class(bytes_received: int, elapsed: float, last: float, timeout: float) -> str:
    if bytes_received == 0:
        return "timeout-no-output"
    recent = max(5.0, min(30.0, timeout * 0.15))
    return (
        "timeout-slow-progress"
        if elapsed - last <= recent
        else "timeout-stalled"
    )


def base_result(args: argparse.Namespace, cpus: int, mem: str) -> dict[str, object]:
    return {
        "schema_version": 1,
        "status": "error",
        "classification": "harness-error",
        "reason": "not started",
        "arch": args.arch,
        "profile": args.profile,
        "smp": cpus,
        "memory": mem,
        "timeout_seconds": args.timeout,
        "git_head": git_head(),
        "build_seconds": 0.0,
        "boot_seconds": 0.0,
        "total_seconds": 0.0,
        "current_phase": "build",
        "first_incomplete_phase": None,
        "completed_phases": [],
        "phase_seconds": {},
        "marker_seconds": {},
        "missing_markers": [],
        "optional_missing_evidence": [],
        "optional_marker_seconds": {},
        "last_progress_seconds": 0.0,
        "last_progress_age_seconds": 0.0,
        "bytes_received": 0,
        "last_serial_lines": [],
        "qemu_command": [],
        "qemu_returncode": None,
        "harness_terminated_qemu": False,
        "serial_log": None,
    }


def main() -> int:
    args = parse_args()
    if args.timeout <= 0 or args.failure_drain_seconds < 0:
        raise RuntimeError("invalid timeout")
    cpus = int(os.environ.get("SMP", "1"))
    if cpus <= 0:
        raise RuntimeError("SMP must be greater than zero")
    mem = os.environ.get("MEM", "256M")
    log_path = args.log or ROOT_DIR / "build" / args.arch / "smoke.log"
    result_path = args.result_json or ROOT_DIR / "build" / args.arch / "smoke-result.json"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    result = base_result(args, cpus, mem)
    result["serial_log"] = str(log_path)
    total_start = time.monotonic()

    try:
        build_start = time.monotonic()
        kernel = args.kernel
        if not args.skip_build and not os.environ.get("SMOKE_COMMAND"):
            env = os.environ.copy()
            env["ARCH"], env["PROFILE"] = args.arch, args.profile
            subprocess.run([str(ROOT_DIR / "scripts/build.sh")], cwd=ROOT_DIR, env=env, check=True)
        if not os.environ.get("SMOKE_COMMAND"):
            if kernel is None:
                kernel = Path((ROOT_DIR / "build" / args.arch / "kernel.path").read_text().strip())
            kernel = kernel.resolve()
            if not kernel.is_file():
                raise RuntimeError(f"kernel ELF does not exist: {kernel}")
        result["build_seconds"] = round(time.monotonic() - build_start, 6)

        command = qemu_command(args.arch, kernel)
        result["qemu_command"] = command
        print("smoke command:", shlex.join(command), flush=True)
        print("smoke log:", log_path, flush=True)
        print("smoke result:", result_path, flush=True)

        tracker = MarkerTracker(required_markers(args.arch, args.profile, cpus))
        optional_tracker = MarkerTracker(
            optional_markers(args.arch, args.profile, cpus)
        )
        process = subprocess.Popen(
            command, cwd=ROOT_DIR, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, bufsize=0, start_new_session=True,
        )
        assert process.stdout is not None
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        boot_start, deadline = time.monotonic(), time.monotonic() + args.timeout
        scan, tail = b"", b""
        bytes_received = 0
        success = False
        reason = classification = None
        drain_deadline = None

        try:
            with log_path.open("wb") as log:
                while time.monotonic() < deadline:
                    for key, _ in selector.select(timeout=0.1):
                        chunk = os.read(key.fileobj.fileno(), 4096)
                        if not chunk:
                            continue
                        bytes_received += len(chunk)
                        log.write(chunk)
                        log.flush()
                        if not args.quiet:
                            sys.stdout.write(chunk.decode(errors="replace"))
                            sys.stdout.flush()
                        scan = (scan + chunk)[-MAX_SCAN_BUFFER:]
                        tail = (tail + chunk)[-TAIL_BYTES:]
                        elapsed = time.monotonic() - boot_start
                        for phase in tracker.feed(scan, elapsed):
                            print(
                                f"smoke phase: {phase} PASS at {elapsed:.3f}s",
                                flush=True,
                            )
                        optional_tracker.feed(scan, elapsed)

                        if SUCCESS_MARKER in scan and reason is None:
                            if tracker.remaining:
                                reason = "success marker arrived before required evidence"
                                classification = "missing-evidence"
                                drain_deadline = min(deadline, time.monotonic() + args.failure_drain_seconds)
                            else:
                                success = True
                                break

                        marker = next((m for m in FAILURE_MARKERS if m in scan), None)
                        if marker is not None and reason is None:
                            reason = f"serial output contained failure marker: {marker.decode(errors='replace')}"
                            classification = "kernel-failure"
                            drain_deadline = min(deadline, time.monotonic() + args.failure_drain_seconds)

                    if success:
                        break
                    if reason is not None and drain_deadline is not None and time.monotonic() >= drain_deadline:
                        break
                    if process.poll() is not None:
                        reason = f"QEMU exited before success marker (status {process.returncode})"
                        classification = "qemu-exit"
                        break

                elapsed = time.monotonic() - boot_start
                if not success and reason is None:
                    classification = timeout_class(bytes_received, elapsed, tracker.last_progress_seconds, args.timeout)
                    reason = (
                        f"timeout after {args.timeout:.1f}s; "
                        f"last_observed_phase={tracker.current_phase()} "
                        f"first_incomplete_phase={tracker.first_incomplete_phase()} "
                        f"last_progress_age={elapsed - tracker.last_progress_seconds:.3f}s"
                    )
        finally:
            selector.close()
            terminated, returncode = stop_process(process)

        boot_seconds = time.monotonic() - boot_start
        result.update(
            {
                "status": "pass" if success else "fail",
                "classification": "pass" if success else classification,
                "reason": "all required evidence observed" if success else reason,
                "boot_seconds": round(boot_seconds, 6),
                "total_seconds": round(time.monotonic() - total_start, 6),
                "current_phase": tracker.current_phase(),
                "first_incomplete_phase": tracker.first_incomplete_phase(),
                "completed_phases": tracker.completed(),
                "phase_seconds": {k: round(v, 6) for k, v in tracker.phase_seconds.items()},
                "marker_seconds": {k: round(v, 6) for k, v in tracker.marker_seconds.items()},
                "missing_markers": tracker.missing(),
                "optional_missing_evidence": optional_tracker.missing(),
                "optional_marker_seconds": {
                    key: round(value, 6)
                    for key, value in optional_tracker.marker_seconds.items()
                },
                "last_progress_seconds": round(tracker.last_progress_seconds, 6),
                "last_progress_age_seconds": round(max(0.0, boot_seconds - tracker.last_progress_seconds), 6),
                "bytes_received": bytes_received,
                "last_serial_lines": tail.decode(errors="replace").splitlines()[-40:],
                "qemu_returncode": returncode,
                "harness_terminated_qemu": terminated,
            }
        )
        write_json(result_path, result)
        if success:
            optional_missing = optional_tracker.missing()
            if optional_missing:
                print(
                    f"smoke {args.arch}: optional evidence missing: "
                    f"{len(optional_missing)} line(s); see result.json",
                    flush=True,
                )
            print(
                f"smoke {args.arch}: PASS (boot={boot_seconds:.3f}s)",
                flush=True,
            )
            return 0
        print(f"smoke {args.arch}: FAIL: {reason}", file=sys.stderr)
        print(f"classification: {classification}", file=sys.stderr)
        print(f"last observed phase: {tracker.current_phase()}", file=sys.stderr)
        print(
            f"first incomplete phase: {tracker.first_incomplete_phase()}",
            file=sys.stderr,
        )
        print(f"result json: {result_path}", file=sys.stderr)
        return 1
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        result.update(
            {
                "status": "error",
                "classification": "build-error" if isinstance(error, subprocess.CalledProcessError) else "harness-error",
                "reason": str(error),
                "total_seconds": round(time.monotonic() - total_start, 6),
            }
        )
        write_json(result_path, result)
        print(f"smoke test failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
