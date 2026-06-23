#!/usr/bin/env python3
from __future__ import annotations
import shutil, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKIP_PARTS = {"deps", "build", "incremental", ".git", ".cargo", ".oscomp_patch_backup"}

def run(cmd):
    try:
        return subprocess.check_output(cmd, cwd=ROOT, text=True, stderr=subprocess.STDOUT)
    except Exception:
        return ""

def is_elf(p: Path) -> bool:
    try:
        with p.open("rb") as f:
            return f.read(4) == b"\x7fELF"
    except Exception:
        return False

def file_info(p: Path) -> str:
    return run(["file", str(p)]).strip()

def readelf_machine(p: Path) -> str:
    out = run(["readelf", "-h", str(p)])
    for line in out.splitlines():
        if "Machine:" in line:
            return line.split("Machine:", 1)[1].strip()
    return ""

def candidates():
    result = []
    for p in ROOT.rglob("*"):
        if not p.is_file():
            continue
        if any(part in SKIP_PARTS for part in p.parts):
            continue
        if p.name in {"kernel-rv", "kernel-la"}:
            continue
        if p.suffix in {".rlib", ".rmeta", ".o", ".a", ".so", ".d", ".S", ".rs", ".md"}:
            continue
        try:
            size = p.stat().st_size
            if size < 16 * 1024:
                continue
        except OSError:
            continue
        if is_elf(p):
            rel = str(p.relative_to(ROOT))
            result.append((p, rel, file_info(p), readelf_machine(p), p.stat().st_mtime, size))
    return result

def score(item, arch):
    p, rel, info, mach, mtime, size = item
    text = " ".join([rel, info, mach, p.name]).lower()
    s = 0
    if arch == "rv":
        if "risc-v" in text or "riscv" in text:
            s += 1000
        if "riscv64" in text or "rv" in p.name.lower():
            s += 200
    else:
        if "loongarch" in text:
            s += 1000
        if "loong" in text or "la" in p.name.lower():
            s += 200
    if "release" in text:
        s += 80
    if "kernel" in text or "myos" in text or "sudo" in text:
        s += 40
    s += min(size // 65536, 100)
    s += int(mtime) % 100
    return s

def copy_if_needed(src: Path, dst: Path):
    if dst.exists() and dst.stat().st_size > 0:
        return
    shutil.copy2(src, dst)
    try:
        dst.chmod(dst.stat().st_mode | 0o755)
    except Exception:
        pass
    print(f"[oscomp] copied {src.relative_to(ROOT)} -> {dst.name}")

def main():
    items = candidates()
    if not items:
        print("[oscomp] ERROR: no ELF candidates found", file=sys.stderr)
        sys.exit(3)
    print("[oscomp] ELF candidates:")
    for p, rel, info, mach, mtime, size in sorted(items, key=lambda x: x[1])[:80]:
        print(f"  - {rel}: {mach or info}")

    rv_items = sorted(items, key=lambda x: score(x, "rv"), reverse=True)
    la_items = sorted(items, key=lambda x: score(x, "la"), reverse=True)
    if rv_items and score(rv_items[0], "rv") > 0:
        copy_if_needed(rv_items[0][0], ROOT / "kernel-rv")
    if la_items and score(la_items[0], "la") > 0:
        copy_if_needed(la_items[0][0], ROOT / "kernel-la")

    missing = []
    for name in ["kernel-rv", "kernel-la"]:
        p = ROOT / name
        if not p.exists() or p.stat().st_size == 0:
            missing.append(name)
    if missing:
        print(f"[oscomp] ERROR: missing {', '.join(missing)}", file=sys.stderr)
        print("[oscomp] Hint: adjust Makefile.project build targets or scripts/oscomp-collect-kernels.py scoring.", file=sys.stderr)
        sys.exit(4)

if __name__ == "__main__":
    main()
