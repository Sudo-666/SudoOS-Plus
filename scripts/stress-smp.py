#!/usr/bin/env python3
"""Reproducible SMP stress runner with structured per-case artifacts."""

from __future__ import annotations

import argparse, hashlib, itertools, json, os, random, shlex, statistics, subprocess, sys, time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

ROOT_DIR = Path(__file__).resolve().parent.parent


def words(name: str, default: str) -> list[str]:
    return shlex.split(os.environ.get(name, default))


def boolean(name: str, default: bool) -> bool:
    value = os.environ.get(name)
    if value is None:
        return default
    if value.lower() in {"1", "true", "yes", "on"}:
        return True
    if value.lower() in {"0", "false", "no", "off"}:
        return False
    raise RuntimeError(f"{name} must be boolean")


def stamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def head() -> str:
    return subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT_DIR, text=True,
        stdout=subprocess.PIPE, check=True,
    ).stdout.strip()


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def p95(values: list[float]) -> float:
    return sorted(values)[round((len(values) - 1) * 0.95)] if values else 0.0


@dataclass(frozen=True)
class Case:
    arch: str
    smp: str
    mem: str
    profile: str
    loop: int


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--arches", nargs="+", default=words("STRESS_ARCHES", os.environ.get("ARCH", "riscv64")))
    parser.add_argument("--smps", nargs="+", default=words("STRESS_SMPS", os.environ.get("SMP", "4")))
    parser.add_argument("--mems", nargs="+", default=words("STRESS_MEMS", os.environ.get("MEM", "256M")))
    parser.add_argument("--profiles", nargs="+", default=words("STRESS_PROFILES", os.environ.get("PROFILE", "debug")))
    parser.add_argument("--loops", type=int, default=int(os.environ.get("STRESS_LOOPS", "1")))
    parser.add_argument("--timeout", type=float, default=float(os.environ["STRESS_TIMEOUT"]) if "STRESS_TIMEOUT" in os.environ else None)
    parser.add_argument("--timeout-riscv64", type=float, default=float(os.environ.get("STRESS_TIMEOUT_RISCV64", "180")))
    parser.add_argument("--timeout-loongarch64", type=float, default=float(os.environ.get("STRESS_TIMEOUT_LOONGARCH64", "240")))
    parser.add_argument("--seed", default=os.environ.get("STRESS_SEED", stamp()))
    parser.add_argument("--log-root", type=Path, default=Path(os.environ.get("STRESS_LOG_DIR", ROOT_DIR / "build/stress-smp")))
    parser.add_argument("--keep-going", action=argparse.BooleanOptionalAction, default=boolean("STRESS_KEEP_GOING", False))
    parser.add_argument("--retry-timeouts", action=argparse.BooleanOptionalAction, default=boolean("STRESS_RETRY_TIMEOUTS", True))
    parser.add_argument("--retry-factor", type=float, default=float(os.environ.get("STRESS_RETRY_FACTOR", "2")))
    parser.add_argument("--fail-on-flaky", action=argparse.BooleanOptionalAction, default=boolean("STRESS_FAIL_ON_FLAKY", False))
    return parser.parse_args()


def run_logged(command: list[str], env: dict[str, str], log: Path) -> int:
    print("$", shlex.join(command), flush=True)
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("w") as output:
        process = subprocess.Popen(command, cwd=ROOT_DIR, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, bufsize=1)
        assert process.stdout is not None
        for line in process.stdout:
            sys.stdout.write(line)
            output.write(line)
            output.flush()
        return process.wait()


def build_all(args: argparse.Namespace, run_dir: Path) -> dict[tuple[str, str], Path]:
    kernels = {}
    for profile, arch in itertools.product(args.profiles, args.arches):
        env = os.environ.copy()
        env.update({"ARCH": arch, "PROFILE": profile})
        code = run_logged([str(ROOT_DIR / "scripts/build.sh")], env, run_dir / "builds" / f"{arch}-{profile}.log")
        if code:
            raise RuntimeError(f"build failed: {arch}/{profile}")
        kernel = Path((ROOT_DIR / "build" / arch / "kernel.path").read_text().strip()).resolve()
        if not kernel.is_file():
            raise RuntimeError(f"missing kernel: {kernel}")
        kernels[(arch, profile)] = kernel
    return kernels


def timeout_for(args: argparse.Namespace, arch: str) -> float:
    return args.timeout if args.timeout is not None else (args.timeout_riscv64 if arch == "riscv64" else args.timeout_loongarch64)


def run_attempt(case: Case, case_dir: Path, attempt: int, timeout: float, kernel: Path):
    attempt_dir = case_dir / f"attempt-{attempt}"
    result_path = attempt_dir / "result.json"
    env = os.environ.copy()
    env.update({"ARCH": case.arch, "SMP": case.smp, "MEM": case.mem, "PROFILE": case.profile})
    command = [
        sys.executable, str(ROOT_DIR / "scripts/smoke.py"),
        "--arch", case.arch, "--profile", case.profile,
        "--timeout", str(timeout), "--skip-build", "--kernel", str(kernel),
        "--log", str(attempt_dir / "serial.log"),
        "--result-json", str(result_path),
    ]
    code = run_logged(command, env, attempt_dir / "harness.log")
    result = json.loads(result_path.read_text()) if result_path.is_file() else {
        "classification": "missing-result", "reason": "no result.json", "boot_seconds": 0.0
    }
    return code, result


def main() -> int:
    args = parse_args()
    if args.loops <= 0 or args.retry_factor <= 1:
        raise RuntimeError("invalid loop/retry settings")
    commit = head()
    run_dir = args.log_root / f"run-{stamp()}-{commit[:12]}-seed{args.seed}"
    run_dir.mkdir(parents=True)
    (args.log_root / "latest.txt").write_text(str(run_dir) + "\n")
    config = vars(args).copy()
    config["log_root"] = str(config["log_root"])
    config["git_head"] = commit
    write_json(run_dir / "config.json", config)

    kernels = build_all(args, run_dir)
    cases = [
        Case(a, s, m, p, loop)
        for loop in range(1, args.loops + 1)
        for p in args.profiles for m in args.mems for s in args.smps for a in args.arches
    ]
    random.Random(int.from_bytes(hashlib.sha256(args.seed.encode()).digest()[:8], "big")).shuffle(cases)
    results = []
    started = time.monotonic()

    for index, case in enumerate(cases, 1):
        name = f"{index:05d}-{case.arch}-smp{case.smp}-{case.mem}-{case.profile}-loop{case.loop}-seed{args.seed}"
        case_dir = run_dir / "cases" / name
        case_dir.mkdir(parents=True)
        timeout = timeout_for(args, case.arch)
        write_json(case_dir / "config.json", {
            "name": name, "arch": case.arch, "smp": int(case.smp), "mem": case.mem,
            "profile": case.profile, "loop": case.loop, "seed": args.seed,
            "timeout_seconds": timeout, "kernel": str(kernels[(case.arch, case.profile)]),
        })
        print(f"[{index}/{len(cases)}] {name} timeout={timeout}s")
        code, first = run_attempt(case, case_dir, 1, timeout, kernels[(case.arch, case.profile)])
        attempts, final = [first], first
        status = "pass" if code == 0 else "fail"
        classification = str(first.get("classification"))
        if code and args.retry_timeouts and classification.startswith("timeout-"):
            retry_code, retry = run_attempt(case, case_dir, 2, timeout * args.retry_factor, kernels[(case.arch, case.profile)])
            attempts.append(retry)
            final = retry
            status = "flaky-timeout-recovered" if retry_code == 0 else "fail"

        entry = {
            "name": name, "status": status,
            "classification": final.get("classification"),
            "reason": final.get("reason"),
            "boot_seconds": float(final.get("boot_seconds", 0.0)),
            "case_dir": str(case_dir), "attempts": attempts,
        }
        results.append(entry)
        write_json(case_dir / "case-result.json", entry)
        print(status.upper(), name)
        if status == "fail" and not args.keep_going:
            break
        if status == "flaky-timeout-recovered" and args.fail_on_flaky and not args.keep_going:
            break

    failures = [x for x in results if x["status"] == "fail"]
    flaky = [x for x in results if x["status"] == "flaky-timeout-recovered"]
    passed = [x for x in results if x["status"] == "pass"]
    durations = [float(x["boot_seconds"]) for x in results]
    failed = bool(failures) or (args.fail_on_flaky and bool(flaky))
    classes = {}
    for x in results:
        k = str(x["classification"])
        classes[k] = classes.get(k, 0) + 1
    summary = {
        "schema_version": 1, "status": "fail" if failed else "pass",
        "seed": args.seed, "git_head": commit, "run_dir": str(run_dir),
        "planned_case_count": len(cases), "case_count": len(results),
        "pass_count": len(passed), "flaky_count": len(flaky), "fail_count": len(failures),
        "total_seconds": round(time.monotonic() - started, 6),
        "boot_seconds": {
            "median": round(statistics.median(durations), 6) if durations else 0,
            "p95": round(p95(durations), 6), "max": round(max(durations), 6) if durations else 0,
        },
        "classification_counts": classes,
        "slowest_cases": sorted(results, key=lambda x: x["boot_seconds"], reverse=True)[:10],
        "failed_cases": failures, "flaky_cases": flaky, "results": results,
    }
    write_json(run_dir / "summary.json", summary)
    lines = [
        "# SudoOS SMP stress summary", "",
        f"- status: **{summary['status']}**", f"- seed: `{args.seed}`",
        f"- git head: `{commit}`", f"- pass: {len(passed)}",
        f"- flaky: {len(flaky)}", f"- fail: {len(failures)}",
        f"- median boot: {summary['boot_seconds']['median']} s",
        f"- p95 boot: {summary['boot_seconds']['p95']} s",
        f"- max boot: {summary['boot_seconds']['max']} s", "",
        "## Classification counts", "",
    ]
    lines += [f"- `{k}`: {v}" for k, v in sorted(classes.items())]
    (run_dir / "summary.md").write_text("\n".join(lines) + "\n")
    print("stress summary:", run_dir / "summary.md")
    return 1 if failed else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"stress-smp: error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
