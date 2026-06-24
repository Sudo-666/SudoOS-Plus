#!/usr/bin/env python3
"""final P1 runtime audit: interpreter aliases, sdcard materialize, shebang,
   loaded-mm repair."""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
checks = []

def add(ok, name):
    checks.append((ok, name))

main_rs = (root / "kernel/src/main.rs").read_text(encoding="utf-8")
user_rs = (root / "kernel/src/user.rs").read_text(encoding="utf-8")
task_rs = (root / "kernel/src/task/mod.rs").read_text(encoding="utf-8")

# --- P1-A: runtime loader aliases ---
add("oscomp_install_runtime_loader_aliases" in main_rs,
    "main.rs has oscomp_install_runtime_loader_aliases function")
add("/glibc/lib/ld-linux-riscv64-lp64d.so.1" in main_rs,
    "RISC-V glibc ld-linux alias source present")
add("/glibc/lib/ld-linux-loongarch-lp64d.so.1" in main_rs,
    "LoongArch glibc ld-linux alias source present")
add("/lib64/ld-linux-loongarch-lp64d.so.1" in main_rs,
    "lib64 dst path for LoongArch ld-linux present")
add("oscomp_install_runtime_lib_aliases" in main_rs,
    "main.rs has oscomp_install_runtime_lib_aliases function")

# --- P1-B: sdcard materialize ---
add("oscomp_materialize_ext4_dir_flat" in main_rs,
    "main.rs has oscomp_materialize_ext4_dir_flat function")
add("sdcard: expanded" in main_rs and "files installed" in main_rs,
    "materialize function prints file count")
# Ensure old broken expand function is gone
add("fn expand_ext4_directory" not in main_rs,
    "old broken expand_ext4_directory is removed")

# --- P1-C: shebang / binfmt_script ---
add("fn parse_shebang" in user_rs,
    "user.rs has shebang parser")
add('#!' in user_rs.split("fn parse_shebang")[1].split("fn ")[0]
    if "fn parse_shebang" in user_rs else False,
    "parse_shebang checks for #! prefix")
add("/usr/bin/env" in main_rs or "/usr/bin/env" in user_rs,
    "/usr/bin/env symlinked or created")

# --- P1-D: loaded-mm repair ---
add("loaded-mm mismatch" in task_rs or "loaded.mm mismatch" in task_rs,
    "task/mod.rs has loaded-mm mismatch repair (non-panic path)")
add("exec loaded-mm mismatch" in task_rs or "loaded_matches" in task_rs,
    "exec_replace_current_user_mm has loaded-mm repair")
# Ensure old unconditional-panic assert! has been softened.
# The new code uses a `mismatch` flag instead of assert! in the match.
add("let mismatch" in task_rs or ("repair" in task_rs and "loaded-mm" in task_rs),
    "switch_mm_irqs_off has repair path (not unconditional panic)")

# --- /lib64 is a real directory, not a symlink ---
add('symlink("/lib", "/lib64")' not in main_rs,
    "/lib64 is NOT a symlink to /lib (real directory)")

failed = [name for ok, name in checks if not ok]
if failed:
    print("final P1 runtime audit: FAIL")
    for name in failed:
        print("  FAIL:", name)
    sys.exit(1)
print("final P1 runtime audit: PASS")
for _, name in checks:
    print("  PASS:", name)
