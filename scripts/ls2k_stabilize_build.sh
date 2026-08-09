#!/usr/bin/env bash
# ls2k_stabilize: build one LoongArch platform and refresh the repo-root artifact.
#   $1 = ls2k1000 | qemu-virt   (default: ls2k1000)
#
# The vendored alloc crate is compiled via -Z build-std into a hash-keyed rlib
# that cargo does NOT always invalidate when vendor/rust-src changes. We stamp
# the vendored alloc.rs sha256 and delete any stale liballoc rlib when it moves.
set -euo pipefail

export PATH=/root/.rustup/toolchains/nightly-2025-01-18-x86_64-unknown-linux-gnu/bin:$PATH
cd /mnt/d/oskernel2026-0xdeadbeef

PLATFORM="${1:-ls2k1000}"
case "$PLATFORM" in
  ls2k1000) ART=kernel-ls2k1000 ;;
  qemu-virt) ART=kernel-la ;;
  *) echo "usage: $0 ls2k1000|qemu-virt" >&2; exit 2 ;;
esac

# ---- invalidate stale build-std alloc rlib if vendored source moved ----
DEPS=build/loongarch64/cargo/loongarch64-unknown-none-softfloat/release/deps
STAMP=build/loongarch64/.alloc_src_stamp
ALLOC_SRC=vendor/rust-src/library/alloc/src/alloc.rs
CUR_HASH=$(sha256sum "$ALLOC_SRC" | cut -d' ' -f1)
if [ ! -f "$STAMP" ] || [ "$(cat "$STAMP" 2>/dev/null || true)" != "$CUR_HASH" ]; then
    echo "== vendored alloc source changed; dropping stale liballoc rlib =="
    rm -f "$DEPS"/liballoc-*.rlib
    echo "$CUR_HASH" > "$STAMP"
fi

echo "== build $PLATFORM =="
ARCH=loongarch64 PLATFORM=$PLATFORM PROFILE=release ./scripts/build.sh
cp build/loongarch64/cargo/loongarch64-unknown-none-softfloat/release/myos-kernel "$ART"

echo "== artifact =="
sha256sum "$ART"
ls -l "$ART"
echo "BUILD_OK $PLATFORM"
