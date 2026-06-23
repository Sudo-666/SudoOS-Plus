#!/usr/bin/env python3
from pathlib import Path
import sys

root = Path.cwd()
bad = []
mentions = []
for base in [root / "arch" / "riscv64", root / "kernel", root / "boot"]:
    if not base.exists():
        continue
    for path in base.rglob("*.rs"):
        text = path.read_text(errors="ignore")
        low = text.lower()
        if "low boot mapping" in low:
            mentions.append(path.relative_to(root))
            if "low boot mapping: removed" in low:
                bad.append(path.relative_to(root))

if bad:
    print("[oscomp-riscv-highhalf-audit] FAIL: dangerous low boot mapping removal text remains")
    for p in bad:
        print("  ", p)
    sys.exit(1)

if not mentions:
    print("[oscomp-riscv-highhalf-audit] WARN: no low boot mapping status source found")
else:
    print("[oscomp-riscv-highhalf-audit] PASS: low boot mapping removal status disabled/retained")
    for p in mentions:
        print("  ", p)
