#!/usr/bin/env bash
set -euo pipefail

# OSKernel2026: ensure -Z build-std has a complete rust-src in the active sysroot.
if [ -x "./scripts/oscomp-prepare-rust-src.sh" ]; then
    ./scripts/oscomp-prepare-rust-src.sh
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TOOLCHAIN="${OSCOMP_TOOLCHAIN:-nightly-2025-01-18}"
VENDOR_DIR="${OSCOMP_VENDOR_DIR:-vendor/cargo}"
TMP_VENDOR="${VENDOR_DIR}.tmp.$$"
TMP_CONFIG="$(mktemp "${TMPDIR:-/tmp}/oscomp-cargo-config.XXXXXX")"
BACKUP_CONFIG=""

cleanup() {
  rm -f "$TMP_CONFIG"
  rm -rf "$TMP_VENDOR"
  if [ -n "$BACKUP_CONFIG" ] && [ -f "$BACKUP_CONFIG" ]; then
    mkdir -p .cargo
    mv "$BACKUP_CONFIG" .cargo/config.toml
  fi
}
trap cleanup EXIT

log() { printf '[oscomp-vendor] %s\n' "$*"; }

if [ -x scripts/oscomp-fix-cargo-toml10.py ]; then
  log "normalizing Cargo.toml files for old contest Cargo"
  python3 scripts/oscomp-fix-cargo-toml10.py
fi

log "toolchain: $TOOLCHAIN"
if ! rustup run "$TOOLCHAIN" rustc --version >/dev/null 2>&1; then
  cat >&2 <<EOF
[oscomp-vendor] ERROR: Rust toolchain '$TOOLCHAIN' is not installed.
Install it locally before vendoring, for example:
  rustup toolchain install $TOOLCHAIN
  rustup component add rust-src --toolchain $TOOLCHAIN
EOF
  exit 1
fi

SYSROOT="$(rustup run "$TOOLCHAIN" rustc --print sysroot)"
SYSROOT_MANIFEST="$SYSROOT/lib/rustlib/src/rust/library/Cargo.toml"
if [ ! -f "$SYSROOT_MANIFEST" ]; then
  cat >&2 <<EOF
[oscomp-vendor] ERROR: rust-src for '$TOOLCHAIN' is missing.
The contest build uses '-Z build-std=core,alloc', so vendor must include
sysroot crate dependencies such as compiler_builtins.
Run locally:
  rustup component add rust-src --toolchain $TOOLCHAIN
Then rerun:
  make oscomp-vendor
EOF
  exit 1
fi
log "sysroot manifest: $SYSROOT_MANIFEST"

# cargo vendor must be allowed to read crates.io while creating vendor/cargo.
# If the previous submission patch already copied cargo-dot/config.toml into
# .cargo/config.toml, that source replacement would make cargo vendor look only
# in the incomplete old vendor directory.  Temporarily move it away.
if [ -f .cargo/config.toml ]; then
  mkdir -p build/oscomp
  BACKUP_CONFIG="build/oscomp/config.toml.before-vendor.$$"
  mv .cargo/config.toml "$BACKUP_CONFIG"
fi

mkdir -p "$(dirname "$VENDOR_DIR")" cargo-dot build/oscomp
rm -rf "$TMP_VENDOR"

log "vendoring project crates plus build-std/sysroot crates"
set +e
cargo +"$TOOLCHAIN" vendor --versioned-dirs --sync "$SYSROOT_MANIFEST" "$TMP_VENDOR" > "$TMP_CONFIG"
status=$?
set -e
if [ "$status" -ne 0 ]; then
  cat >&2 <<EOF
[oscomp-vendor] ERROR: cargo vendor failed.
This command intentionally syncs the Rust sysroot manifest so that
compiler_builtins and other build-std crates are present in vendor/cargo.
If this was an offline local machine, connect once and rerun make oscomp-vendor.
EOF
  exit "$status"
fi

rm -rf "$VENDOR_DIR"
mv "$TMP_VENDOR" "$VENDOR_DIR"

# Keep the submitted Cargo config in a non-hidden directory because the contest
# clone filter removes hidden files/directories.  scripts/oscomp-build.sh copies
# this back to .cargo/config.toml before building.
cat > cargo-dot/config.toml <<'EOF'
# Copied to .cargo/config.toml by scripts/oscomp-build.sh.
# Keep this directory non-hidden because the contest clone filter removes hidden dirs.

[build]
target-dir = "target/oscomp"

[net]
git-fetch-with-cli = true

[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor/cargo"
EOF

# Restore local .cargo/config.toml so the current checkout can build exactly as
# the contest clone will build.  The backup is restored by cleanup if present;
# here we intentionally replace it with the fresh non-hidden config.
if [ -n "$BACKUP_CONFIG" ] && [ -f "$BACKUP_CONFIG" ]; then
  rm -f "$BACKUP_CONFIG"
  BACKUP_CONFIG=""
fi
mkdir -p .cargo
cp cargo-dot/config.toml .cargo/config.toml

if [ ! -d "$VENDOR_DIR/compiler_builtins" ] && ! find "$VENDOR_DIR" -maxdepth 1 -type d -name 'compiler_builtins-*' | grep -q .; then
  cat >&2 <<EOF
[oscomp-vendor] ERROR: vendor completed but compiler_builtins is still missing.
Check whether '$SYSROOT_MANIFEST' is the correct rust-src manifest for the
selected toolchain.
EOF
  exit 1
fi

count="$(find "$VENDOR_DIR" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')"
log "vendored crates: $count"
log "compiler_builtins: present"
log "done"
