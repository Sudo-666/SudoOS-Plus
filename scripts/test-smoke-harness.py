#!/usr/bin/env python3
"""Synthetic process fault injection for scripts/smoke.py."""

from __future__ import annotations
import importlib.util, json, os, subprocess, sys, tempfile
from pathlib import Path

ROOT_DIR = Path(__file__).resolve().parent.parent
SMOKE = ROOT_DIR / "scripts/smoke.py"

EMITTER = r"""
import json, sys, time
from pathlib import Path
mode = sys.argv[1]
markers = Path(sys.argv[2])
if mode == "pass":
    for marker in json.loads(markers.read_text()):
        print(marker, flush=True)
    print("SMOKE_TEST: PASS", flush=True)
    time.sleep(10)
elif mode == "panic":
    print("KERNEL PANIC", flush=True)
    print("panic diagnostic tail", flush=True)
    time.sleep(10)
elif mode == "premature-success":
    print("SMOKE_TEST: PASS", flush=True)
    time.sleep(10)
elif mode == "no-output":
    time.sleep(10)
elif mode == "exit":
    print("runtime pgtbl : active hardware root", flush=True)
    raise SystemExit(7)
else:
    raise SystemExit("unknown mode: " + mode)
"""


def load_smoke():
    spec = importlib.util.spec_from_file_location("sudoos_smoke", SMOKE)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to import smoke.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def main() -> int:
    smoke = load_smoke()
    with tempfile.TemporaryDirectory(prefix="sudoos-harness-") as td:
        root = Path(td)
        emitter = root / "emitter.py"
        emitter.write_text(EMITTER)
        markers = root / "markers.json"
        markers.write_text(json.dumps([
            m.value.decode(errors="replace")
            for m in smoke.required_markers("riscv64", "debug", 4)
        ]))
        expectations = {
            "pass": (0, "pass", 5.0),
            "panic": (1, "kernel-failure", 5.0),
            "premature-success": (1, "missing-evidence", 5.0),
            "no-output": (1, "timeout-no-output", 0.15),
            "exit": (1, "qemu-exit", 5.0),
        }
        for mode, (code_expected, class_expected, timeout) in expectations.items():
            result_path = root / f"{mode}.json"
            env = os.environ.copy()
            env.update({
                "SMP": "4", "MEM": "256M",
                "SMOKE_COMMAND": f"{sys.executable} {emitter} {mode} {markers}",
            })
            command = [
                sys.executable, str(SMOKE), "--arch", "riscv64",
                "--profile", "debug", "--timeout", str(timeout),
                "--failure-drain-seconds", "0.02", "--skip-build", "--quiet",
                "--log", str(root / f"{mode}.log"),
                "--result-json", str(result_path),
            ]
            run = subprocess.run(command, cwd=ROOT_DIR, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=5)
            result = json.loads(result_path.read_text())
            assert run.returncode == code_expected, (mode, run.stdout, result)
            assert result["classification"] == class_expected, (mode, result)
            print(f"harness injection {mode}: {class_expected} verified")
    print("smoke harness fault injection: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
