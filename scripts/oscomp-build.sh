#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly-2025-01-18}"
export PROFILE="${PROFILE:-release}"
export OSCOMP_SUBMIT=1
export SKIP_SMOKE=1
export NO_SMOKE=1
export CI=1

echo "[oscomp] toolchain: $RUSTUP_TOOLCHAIN"
echo "[oscomp] profile  : $PROFILE"

# The judge removes hidden directories. Recreate .cargo from committed cargo-dot.
if [ -d cargo-dot ]; then
  rm -rf .cargo
  mkdir -p .cargo
  cp -R cargo-dot/. .cargo/
fi

if [ -d vendor/cargo ]; then
  export CARGO_NET_OFFLINE="${CARGO_NET_OFFLINE:-true}"
  echo "[oscomp] cargo vendor: vendor/cargo present, offline=$CARGO_NET_OFFLINE"
else
  echo "[oscomp] cargo vendor: missing; run 'make oscomp-vendor' before final submit if Cargo has external deps"
fi

rm -f kernel-rv kernel-la disk-rv.img

build_with_known_targets() {
  local mf="Makefile.project"
  if grep -Eq '^kernel-rv:' "$mf" && grep -Eq '^kernel-la:' "$mf"; then
    echo "[oscomp] invoking original kernel-rv/kernel-la targets"
    make -f "$mf" kernel-rv kernel-la PROFILE="$PROFILE" OSCOMP_SUBMIT=1 SKIP_SMOKE=1 NO_SMOKE=1 CI=1 || exit $?
    return 0
  fi
  if grep -Eq '^build-riscv64:' "$mf" && grep -Eq '^build-loongarch64:' "$mf"; then
    echo "[oscomp] invoking original build-riscv64/build-loongarch64 targets"
    make -f "$mf" build-riscv64 build-loongarch64 PROFILE="$PROFILE" OSCOMP_SUBMIT=1 SKIP_SMOKE=1 NO_SMOKE=1 CI=1 || exit $?
    return 0
  fi
  return 1
}

build_with_original_all() {
  echo "[oscomp] invoking original Makefile.project all"
  if command -v timeout >/dev/null 2>&1; then
    timeout 900s make -f Makefile.project all PROFILE="$PROFILE" OSCOMP_SUBMIT=1 SKIP_SMOKE=1 NO_SMOKE=1 CI=1
  else
    make -f Makefile.project all PROFILE="$PROFILE" OSCOMP_SUBMIT=1 SKIP_SMOKE=1 NO_SMOKE=1 CI=1
  fi
}

if ! build_with_known_targets; then
  build_with_original_all
fi

echo "[oscomp] collecting ELF kernels"
python3 scripts/oscomp-collect-kernels.py

# Optional auxiliary disk names. RISC-V judge checks disk.img; LoongArch checks disk-la.img.
if [ -f disk.img ] && [ ! -f disk-la.img ]; then
  cp disk.img disk-la.img
  echo "[oscomp] copied disk.img -> disk-la.img"
fi
if [ -f disk-la.img ] && [ ! -f disk.img ]; then
  cp disk-la.img disk.img
  echo "[oscomp] copied disk-la.img -> disk.img"
fi

test -s kernel-rv || { echo "[oscomp] ERROR: kernel-rv was not produced" >&2; exit 2; }
test -s kernel-la || { echo "[oscomp] ERROR: kernel-la was not produced" >&2; exit 2; }
file kernel-rv kernel-la || true
echo "[oscomp] PASS: required root ELF files are present"
