#!/usr/bin/env bash
# ls2k_stabilize: build one LoongArch platform and refresh the repo-root artifact.
#   $1 = ls2k1000 | qemu-virt   (default: ls2k1000)
# Env: LS2K_RM_ALLOC=1 -> force rebuild of the vendored alloc rlib.
set -euo pipefail

export PATH=/root/.rustup/toolchains/nightly-2025-01-18-x86_64-unknown-linux-gnu/bin:$PATH
cd /mnt/d/oskernel2026-0xdeadbeef

PLATFORM="${1:-ls2k1000}"
case "$PLATFORM" in
  ls2k1000) ART=kernel-ls2k1000 ;;
  qemu-virt) ART=kernel-la ;;
  *) echo "usage: $0 ls2k1000|qemu-virt" >&2; exit 2 ;;
esac

if [ "${LS2K_RM_ALLOC:-0}" = "1" ]; then
  echo "== rm cached alloc rlib =="
  rm -f build/loongarch64/cargo/loongarch64-unknown-none-softfloat/release/deps/liballoc-*.rlib
fi

echo "== build $PLATFORM =="
ARCH=loongarch64 PLATFORM=$PLATFORM PROFILE=release ./scripts/build.sh
cp build/loongarch64/cargo/loongarch64-unknown-none-softfloat/release/myos-kernel "$ART"

echo "== artifact =="
sha256sum "$ART"
ls -l "$ART"
echo "BUILD_OK $PLATFORM"
