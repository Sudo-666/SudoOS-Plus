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

# PR-0 (reproducible build): never skip a divergent sysroot. The previous
# check only tested "library complete" and exited 0, so hand-edits to the
# sysroot's alloc/core (e.g. the LS2K1000 assert_unchecked/sanitize_layout
# experiments) were silently kept and every board image was built from a
# source tree that did not match vendor/rust-src. Now any content or
# presence divergence between vendor/ and the sysroot forces a full
# reinstall, so build-std always compiles exactly the vendored (committed)
# sources.
sync_needed() {
  if ! library_complete "$DEST"; then
    return 0
  fi
  # diff -rq: report only whether any file differs in content or presence.
  if diff -rq "$SRC" "$DEST" >/dev/null 2>&1; then
    return 1 # identical -> no sync
  fi
  return 0
}

if sync_needed; then
  # Always replace the whole library directory: a partial/corrupt install
  # (Cargo.lock present without core/alloc/std) tricks build-std into using
  # a broken sysroot.
  echo "[oscomp-rust-src] syncing vendored rust-src -> sysroot (tree differs or incomplete)"
  mkdir -p "$DEST_PARENT"
  rm -rf "$DEST"
  cp -a "$SRC" "$DEST"
else
  echo "[oscomp-rust-src] sysroot rust-src matches vendored (no sync)"
fi

if ! library_complete "$DEST"; then
  echo "[oscomp-rust-src] ERROR: sysroot rust-src install is still incomplete" >&2
  for f in Cargo.toml Cargo.lock core/Cargo.toml alloc/Cargo.toml std/Cargo.toml; do
    if [[ ! -f "$DEST/$f" ]]; then
      echo "[oscomp-rust-src] missing sysroot library/$f" >&2
    fi
  done
  exit 1
fi

# PR-0: emit the exact alloc/core sources that will be compiled, so a board
# image can be tied back to the source that produced it.
echo "[oscomp-rust-src] vendored alloc.rs   sha256: $(sha256sum "$SRC/alloc/src/alloc.rs" | cut -d' ' -f1)"
echo "[oscomp-rust-src] vendored raw_vec.rs sha256: $(sha256sum "$SRC/alloc/src/raw_vec.rs" | cut -d' ' -f1)"
echo "[oscomp-rust-src] sysroot rust-src ready"
