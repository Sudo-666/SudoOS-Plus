#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
if [ -d vendor/cargo/compiler_builtins ] || find vendor/cargo -maxdepth 1 -type d -name 'compiler_builtins-*' 2>/dev/null | grep -q .; then
  echo "[oscomp-buildstd-audit] PASS: compiler_builtins is vendored"
else
  echo "[oscomp-buildstd-audit] FAIL: compiler_builtins missing from vendor/cargo" >&2
  echo "Run: make oscomp-vendor" >&2
  exit 1
fi
if [ -f cargo-dot/config.toml ] && grep -q 'vendor/cargo' cargo-dot/config.toml; then
  echo "[oscomp-buildstd-audit] PASS: cargo-dot config points at vendor/cargo"
else
  echo "[oscomp-buildstd-audit] FAIL: cargo-dot/config.toml missing vendor source replacement" >&2
  exit 1
fi
