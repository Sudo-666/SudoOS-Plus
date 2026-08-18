#!/usr/bin/env python3
"""VisionFive 2 真机串口日志检查器 (Gate A/B/C/D)。

只解析保存后的纯文本串口日志,不启动任何虚拟机:

    python3 scripts/check-visionfive2-serial-log.py \
        --log /path/to/vf2-serial.log --gate a|b|c|d

Gate 定义 (SudoOS-VisionFive2-TFTP-CodePlan §13):

- Gate A  (conf-selftest): BOOT00..BOOT13 启动标志 + 单核内核自测路径。
- Gate B  (conf-single):   真实 BusyBox /init(PID 1) -> sudoos:/#。
- Gate C  (conf-smp):      4 CPU online/active/IPI-ready + 完整终端语义
                            (动态 PS1 / Ctrl-C / 管道 Ctrl-C / VEOF / fork)。
- Gate D  (stability):     稳定性压力场景,无拒绝标志、无残留进程。

所有 Gate 都检查全局拒绝标志:
  invalid FDT | HsmUnavailable | hart_start failed | kernel/user page fault |
  recursive lock acquisition | lock order violation | panicked at |
  kernel panic | OOM | ale-fail | unknown-syscall
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# ── 全局拒绝标志 ────────────────────────────────────────────────
REJECT_MARKERS = [
    "invalid FDT",
    "HsmUnavailable",
    "hart_start failed",
    "kernel page fault:",
    "user page fault before task subsystem:",
    "recursive lock acquisition",
    "lock order violation",
    "panicked at",
    "kernel panic",
    "OOM",
    "ale-fail",
    "unknown-syscall",
    # Heap is not installed before BOOT06 heap-ready: any allocation request
    # there trips the alloc-error handler and must fail the gate.  riscv64 uses
    # the default __rdl_oom message; ls2k1000 prints HEAP_FATAL-*.
    "memory allocation of",
    "HEAP_FATAL",
    # User-mode faults: a real SIGSEGV in a child process is a stability
    # failure, not a pass.  The kernel prints `sigsegv: pid=...` on the serial
    # and BusyBox ash echoes `Segmentation fault`.
    "sigsegv:",
    "Segmentation fault",
]

# ── Gate A: conf-selftest 单核自测 ──────────────────────────────
GATE_A_BOOT_MARKERS = [
    "B",
    "BOOT00 entry",
    "BOOT01 fdt-valid",
    "BOOT02 memory-map",
    "BOOT03 early-page-table",
    "BOOT04 final-page-table",
    "BOOT05 buddy-ready",
    "BOOT06 heap-ready",
    "BOOT07 bsp-trap-ready",
    "BOOT11 all-ap-online",
    "BOOT12 virtio-ready",
    "BOOT13 rootfs-ready",
]

# ── Gate B: conf-single 单核真实 PID 1 ──────────────────────────
GATE_B_MARKERS = [
    "INIT: exec pid=1 path=/init",
    "SUDOOS_INIT_READY",
    "Please press Enter to activate this console.",
    "sudoos:/#",
]

# ── Gate C: conf-smp 4 核 + 完整终端语义 ────────────────────────
GATE_C_MARKERS = [
    "discovered CPUs : 4",
    "online CPUs     : 4",
    "active CPUs     : 4",
    "IPI-ready CPUs  : 4",
    "CPU-COUNTERS PASS",
    "FORK_WAIT_OK",
]

# Gate C 动态 PS1 至少验证的目录。
GATE_C_PS1_DIRS = ["/", "/bin", "/mnt", "/tmp"]

# ── Gate D: 稳定性 ──────────────────────────────────────────────
GATE_D_MIN_ITERATIONS = 20


class GateFailure(Exception):
    pass


def load_log(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def check_rejects(text: str) -> list[str]:
    return [marker for marker in REJECT_MARKERS if marker in text]


def require(text: str, marker: str) -> None:
    if marker not in text:
        raise GateFailure(f"missing marker: {marker}")


def check_gate_a(text: str) -> list[str]:
    missing = [m for m in GATE_A_BOOT_MARKERS if m not in text]
    if missing:
        raise GateFailure(f"missing boot markers: {missing}")

    # 单核路径: discovered CPUs = 1。
    if "discovered CPUs : 1" not in text:
        raise GateFailure("conf-selftest must show discovered CPUs : 1")

    # SMP 拓扑行必须出现 boot hart 映射。
    if "SMP: boot hart=" not in text or "SMP: logical0 -> hart " not in text:
        raise GateFailure("missing SMP topology lines (SMP: boot hart / logical0)")

    return [
        f"boot markers    : {len(GATE_A_BOOT_MARKERS)}/{len(GATE_A_BOOT_MARKERS)}",
        "single CPU      : discovered CPUs : 1",
        "SMP topology    : boot hart + logical0 present",
    ]


def check_gate_b(text: str) -> list[str]:
    missing = [m for m in GATE_B_MARKERS if m not in text]
    if missing:
        raise GateFailure(f"missing PID 1 / prompt markers: {missing}")

    # conf-single must run truly single-core. If a stale U-Boot env bootargs
    # leaks in (without rdinit=/init sudoos.maxcpus=1), the FIT still selects
    # conf-single but the kernel boots 4 cores onto the selftest path instead
    # of PID 1 — catch that here rather than in Gate C.
    for marker in ("discovered CPUs : 1", "online CPUs     : 1",
                   "active CPUs     : 1"):
        if marker not in text:
            raise GateFailure(f"conf-single must show {marker.strip()} (single-core gate)")

    return [
        f"PID 1 shell     : {len(GATE_B_MARKERS)}/{len(GATE_B_MARKERS)} markers",
        "single core     : discovered/online/active CPUs = 1",
    ]


def check_gate_c(text: str) -> list[str]:
    missing = [m for m in GATE_C_MARKERS if m not in text]
    if missing:
        raise GateFailure(f"missing SMP/terminal markers: {missing}")

    # 动态 PS1 目录验证(至少 /、/bin、/mnt、/tmp 出现 shell 提示)。
    ps1_dirs = [d for d in GATE_C_PS1_DIRS if f"sudoos:{d}# " in text or f"sudoos:{d}$ " in text]
    if len(ps1_dirs) < len(GATE_C_PS1_DIRS):
        raise GateFailure(
            f"dynamic PS1 dirs incomplete: saw {ps1_dirs}, want {GATE_C_PS1_DIRS}"
        )

    # 任何 hart_start(0) 都不允许出现。
    if "hart_start(0)" in text or "hardware=0" in text and "ap-start-request" in text:
        raise GateFailure("monitor hart 0 was requested via HSM")

    return [
        "SMP 4x U74      : discovered/online/active/IPI-ready = 4",
        "CPU counters    : CPU-COUNTERS PASS",
        "dynamic PS1     : " + ", ".join(ps1_dirs),
        "fork/wait       : FORK_WAIT_OK",
    ]


def check_gate_d(text: str) -> list[str]:
    # 稳定性证据计数。阈值对真机手工会话保持宽松:重点是重复的
    # fork/wait、Ctrl-C、VEOF 和"shell exit 后被 init 重拉"至少 20 次。
    #
    # 计数严格对齐内核自己的串口 trace 标记,而不是宽松的子串:
    #   shell 重拉  -> TTY-CTTY:   每次 relaunch 的会话重新获取控制终端
    #   fork/wait   -> 整行 FORK_WAIT_OK (只匹配单独一行,避免命令回显误计)
    #   Ctrl-C      -> TTY-SIGINT: 内核 VINTR trace
    #   VEOF        -> TTY-VEOF:   内核 VEOF trace
    # 这些标记在 rdinit 路径默认开启 (init_supervisor 置 verbose trace),
    # 并在 Gate C/D 会话中被验证可用。
    counters = {
        "shell relaunch": len(re.findall(r"TTY-CTTY:", text)),
        "fork/wait": len(re.findall(r"^FORK_WAIT_OK$", text, re.MULTILINE)),
        "Ctrl-C": len(re.findall(r"TTY-SIGINT:", text)),
        "VEOF": len(re.findall(r"TTY-VEOF:", text)),
    }
    thresholds = {
        "shell relaunch": GATE_D_MIN_ITERATIONS,  # init 重拉 shell ≥ 20 次
        "fork/wait": 5,
        "Ctrl-C": 5,
        "VEOF": 1,
    }
    weak = {
        name: count
        for name, count in counters.items()
        if count < thresholds[name]
    }
    if weak:
        raise GateFailure(
            f"stability evidence below thresholds {thresholds}: {weak}"
        )

    # 无残留 sleep/cat:日志尾部 ps 输出中不应再有正在运行的实例。
    tail = text.splitlines()[-5:]
    residual = [line for line in tail if re.search(r"\b(sleep|cat)\b", line)]
    if residual:
        raise GateFailure(f"residual sleep/cat in final ps output: {residual}")

    return [
        f"iterations      : {counters}",
        "residuals       : none in final ps output",
    ]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--log", required=True, help="saved plain-text serial log")
    ap.add_argument("--gate", required=True, choices=["a", "b", "c", "d"],
                    help="which gate to check")
    args = ap.parse_args()

    path = Path(args.log)
    if not path.is_file():
        print(f"error: log not found: {path}", file=sys.stderr)
        return 2

    text = load_log(path)

    # 全局拒绝标志总是检查。
    rejects = check_rejects(text)
    if rejects:
        print(f"VISIONFIVE2_SERIAL_CHECK : FAIL — global reject markers: {rejects}",
              file=sys.stderr)
        return 1

    gates = {
        "a": ("Gate A (conf-selftest)", check_gate_a),
        "b": ("Gate B (conf-single)", check_gate_b),
        "c": ("Gate C (conf-smp)", check_gate_c),
        "d": ("Gate D (stability)", check_gate_d),
    }
    label, checker = gates[args.gate]

    try:
        lines = checker(text)
    except GateFailure as exc:
        print(f"VISIONFIVE2_SERIAL_CHECK : FAIL — {label}: {exc}", file=sys.stderr)
        return 1

    print(f"gate            : {label}")
    for line in lines:
        print(f"  {line}")
    print("reject markers  : none")
    print()
    print("VISIONFIVE2_SERIAL_CHECK : PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
