#!/usr/bin/env bash
# ls2k_stabilize: run the LoongArch QEMU smoke test for kernel-la.
set -euo pipefail
export PATH=/root/.rustup/toolchains/nightly-2025-01-18-x86_64-unknown-linux-gnu/bin:$PATH
cd /mnt/d/oskernel2026-0xdeadbeef
PLATFORM=qemu-virt MEM="${MEM:-1G}" SMP="${SMP:-1}" ./scripts/smoke.py --arch loongarch64 --profile release --timeout "${SMOKE_TIMEOUT:-90}"
