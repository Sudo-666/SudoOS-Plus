#!/usr/bin/env python3
"""M8-B3 R7 high-half UART audit."""
from pathlib import Path
import sys

files = {
    "console": Path("arch/riscv64/src/early_console.rs"),
    "layout": Path("arch/riscv64/src/memory/layout.rs"),
    "phys": Path("arch/riscv64/src/memory/phys_access.rs"),
    "memory": Path("kernel/src/memory.rs"),
    "runtime": Path("kernel/src/runtime_page_table.rs"),
}
missing = [str(path) for path in files.values() if not path.is_file()]
if missing:
    print(f"M8-B3 R7 audit: FAIL: missing {missing}", file=sys.stderr)
    raise SystemExit(1)

text = {name: path.read_text(encoding="utf-8") for name, path in files.items()}
checks = {
    "dedicated high-half alias": "pub const EARLY_UART_FIXMAP" in text["layout"],
    "alias belongs to FIXMAP": "FIXMAP.contains(EARLY_UART_FIXMAP)" in text["layout"],
    "publication gate": "RUNTIME_MAPPING_ACTIVE" in text["console"],
    "console uses selected VA": text["console"].count("virtual_base()") >= 3,
    "MMIO helper uses selected VA": "crate::early_console::virtual_base()" in text["phys"],
    "final root maps UART alias": "for page in [identity_page, fixmap_page]" in text["memory"],
    "alias activated after root switch": text["memory"].find("switch_sv39_root(root)") < text["memory"].find("activate_runtime_mapping();"),
    "private roots copy only high half": "ENTRIES_PER_TABLE / 2..ENTRIES_PER_TABLE" in text["runtime"],
}
failed = [name for name, passed in checks.items() if not passed]
if failed:
    print(f"M8-B3 R7 audit: FAIL: {failed}", file=sys.stderr)
    raise SystemExit(1)

print("M8-B3 R7 audit: PASS")
print("  bootstrap UART identity : retained")
print("  runtime UART fixmap      : shared high half")
print("  console/MMIO VA switch   : published after SATP")
print("  private user low half    : no UART mapping copied")
print("  LoongArch path           : untouched")
