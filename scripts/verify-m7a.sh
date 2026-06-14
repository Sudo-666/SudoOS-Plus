#!/usr/bin/env bash
set -euo pipefail

repo="${1:-$PWD}"
cd "$repo"

cargo fmt --all -- --check
git diff --check
make check

SMP=1 SMOKE_TIMEOUT=300 make smoke-riscv64
SMP=4 SMOKE_TIMEOUT=300 make smoke-riscv64
SMP=1 SMOKE_TIMEOUT=360 make smoke-loongarch64
SMP=4 SMOKE_TIMEOUT=360 make smoke-loongarch64
