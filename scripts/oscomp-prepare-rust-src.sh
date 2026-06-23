#!/usr/bin/env bash
set -euo pipefail

# The contest Docker may include the Rust toolchain but not a complete rust-src
# component. Cargo -Z build-std requires:
#   $SYSROOT/lib/rustlib/src/rust/library/Cargo.lock
# This script installs the repository-vendored Rust library source into the
# active sysroot before building. It is source-only and deterministic.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOOLCHAIN="${OSCOMP_TOOLCHAIN:-nightly-2025-01-18}"

if command -v rustup >/dev/null 2>&1; then
    SYSROOT="$(rustup run "$TOOLCHAIN" rustc --print sysroot 2>/dev/null || rustc --print sysroot)"
else
    SYSROOT="$(rustc --print sysroot)"
fi

SRC="$ROOT/vendor/rust-src/library"
DST="$SYSROOT/lib/rustlib/src/rust/library"

if [ ! -f "$SRC/Cargo.lock" ] || [ ! -f "$SRC/Cargo.toml" ]; then
    echo "[oscomp-rust-src] ERROR: vendored rust-src missing: $SRC" >&2
    echo "[oscomp-rust-src] Run install_oscomp_rustsrc_hotfix.py locally and commit vendor/rust-src." >&2
    exit 1
fi

if [ -f "$DST/Cargo.lock" ] && [ -f "$DST/Cargo.toml" ]; then
    echo "[oscomp-rust-src] sysroot rust-src already complete"
    exit 0
fi

echo "[oscomp-rust-src] installing vendored rust-src into sysroot"
mkdir -p "$(dirname "$DST")"
rm -rf "$DST.tmp-oscomp" "$DST"

# cp -a is available in the contest Docker. Avoid symlink-only setup because some
# Cargo/rustup combinations canonicalize sysroot paths during build-std.
cp -a "$SRC" "$DST.tmp-oscomp"
mv "$DST.tmp-oscomp" "$DST"

if [ ! -f "$DST/Cargo.lock" ]; then
    echo "[oscomp-rust-src] ERROR: failed to install Cargo.lock into $DST" >&2
    exit 1
fi
