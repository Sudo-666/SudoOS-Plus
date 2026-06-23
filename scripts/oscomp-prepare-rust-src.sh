#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOOLCHAIN="${OSCOMP_TOOLCHAIN:-nightly-2025-01-18}"
SRC="$REPO_ROOT/vendor/rust-src/library"

need_file() {
  local base="$1"
  local rel="$2"
  [[ -f "$base/$rel" ]]
}

library_complete() {
  local base="$1"
  need_file "$base" Cargo.toml && \
  need_file "$base" Cargo.lock && \
  need_file "$base" core/Cargo.toml && \
  need_file "$base" alloc/Cargo.toml && \
  need_file "$base" std/Cargo.toml
}

if ! library_complete "$SRC"; then
  echo "[oscomp-rust-src] ERROR: vendored rust-src is incomplete: $SRC" >&2
  for f in Cargo.toml Cargo.lock core/Cargo.toml alloc/Cargo.toml std/Cargo.toml; do
    if [[ ! -f "$SRC/$f" ]]; then
      echo "[oscomp-rust-src] missing vendor/rust-src/library/$f" >&2
    fi
  done
  echo "[oscomp-rust-src] Re-run install_oscomp_rustsrc_repair.py on a machine with rust-src installed." >&2
  exit 1
fi

if command -v rustup >/dev/null 2>&1; then
  SYSROOT="$(rustup run "$TOOLCHAIN" rustc --print sysroot)"
else
  SYSROOT="$(rustc --print sysroot)"
fi
DEST="$SYSROOT/lib/rustlib/src/rust/library"
DEST_PARENT="$(dirname "$DEST")"

if library_complete "$DEST"; then
  echo "[oscomp-rust-src] sysroot rust-src already complete"
  exit 0
fi

# Important: remove partial/corrupt installs. A previous script could leave
# Cargo.lock without core/alloc/std, which tricks build-std into using a broken
# sysroot. Always replace the whole library directory if any required member is
# missing.
echo "[oscomp-rust-src] installing complete vendored rust-src into sysroot"
mkdir -p "$DEST_PARENT"
rm -rf "$DEST"
cp -a "$SRC" "$DEST"

if ! library_complete "$DEST"; then
  echo "[oscomp-rust-src] ERROR: sysroot rust-src install is still incomplete" >&2
  for f in Cargo.toml Cargo.lock core/Cargo.toml alloc/Cargo.toml std/Cargo.toml; do
    if [[ ! -f "$DEST/$f" ]]; then
      echo "[oscomp-rust-src] missing sysroot library/$f" >&2
    fi
  done
  exit 1
fi

echo "[oscomp-rust-src] sysroot rust-src installed"
