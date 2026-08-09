#!/usr/bin/env bash
# ls2k_stabilize: package the board artifacts (elf/bin/uImage/buildinfo).
set -euo pipefail
export PATH=/root/.rustup/toolchains/nightly-2025-01-18-x86_64-unknown-linux-gnu/bin:$PATH
cd /mnt/d/oskernel2026-0xdeadbeef
make kernel-ls2k1000 kernel-ls2k1000.elf kernel-ls2k1000.bin kernel-ls2k1000.uImage kernel-ls2k1000.buildinfo
echo "== artifacts =="
ls -lh kernel-ls2k1000 kernel-ls2k1000.elf kernel-ls2k1000.bin kernel-ls2k1000.uImage kernel-ls2k1000.buildinfo
echo "== hashes =="
sha256sum kernel-ls2k1000.elf kernel-ls2k1000.bin kernel-ls2k1000.uImage
echo "PACKAGE_OK"
