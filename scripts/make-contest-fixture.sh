#!/usr/bin/env bash
# Generate a disposable 32 MiB raw ext4 contest fixture.
#
# The fixture mimics the layout of the real judge image that the oscomp
# runners look for, so the storage chain (BlockDevice -> ext4 -> VFS) can be
# validated on QEMU without shipping an official .img. It is intentionally
# NOT committed to git -- regenerate per test run.
#
# Usage:
#   scripts/make-contest-fixture.sh --arch riscv64 --output build/fixtures/contest-rv.ext4
#   scripts/make-contest-fixture.sh --arch loongarch64 --output build/fixtures/contest-la.ext4

set -Eeuo pipefail

FIXTURE_MIB=32
ARCH=""
OUTPUT=""

usage() {
    cat <<'EOF'
Usage: make-contest-fixture.sh --arch <riscv64|loongarch64> --output <path>
EOF
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch)
            ARCH="$2"
            shift 2
            ;;
        --output)
            OUTPUT="$2"
            shift 2
            ;;
        *)
            usage
            ;;
    esac
done

if [[ -z "$ARCH" || -z "$OUTPUT" ]]; then
    usage
fi
case "$ARCH" in
    riscv64 | loongarch64) ;;
    *) echo "error: unsupported --arch '$ARCH'" >&2 && exit 2 ;;
esac

command -v mkfs.ext4 >/dev/null 2>&1 || {
    echo "error: mkfs.ext4 (e2fsprogs) is required" >&2
    exit 2
}
command -v debugfs >/dev/null 2>&1 || {
    echo "error: debugfs (e2fsprogs) is required" >&2
    exit 2
}

OUTPUT_DIR="$(cd -- "$(dirname -- "$OUTPUT")" && pwd)"
OUTPUT_NAME="$(basename -- "$OUTPUT")"
FIXTURE_ABS="$OUTPUT_DIR/$OUTPUT_NAME"
mkdir -p "$OUTPUT_DIR"

STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT

# Marker files + the paths the real judge image carries.
echo "SUDOOS_CONTEST_FIXTURE_V1" >"$STAGING/SUDOOS_CONTEST_FIXTURE"
echo "$ARCH" >"$STAGING/arch"
mkdir -p "$STAGING/glibc" "$STAGING/musl" "$STAGING/work/tgoskits"

cat >"$STAGING/glibc/cagent_testcode.sh" <<'EOF'
#!/bin/sh
cat /SUDOOS_CONTEST_FIXTURE
cat /arch
echo FIXTURE_OSCOMP_PASS
EOF
cp "$STAGING/glibc/cagent_testcode.sh" "$STAGING/musl/cagent_testcode.sh"

cat >"$STAGING/work/tgoskits/Cargo.toml" <<'EOF'
[package]
name = "tgoskits"
version = "0.0.0"
edition = "2021"
EOF

# 32 MiB raw ext4, 4 KiB blocks, no reserved blocks.
dd if=/dev/zero of="$FIXTURE_ABS" bs=1M count="$FIXTURE_MIB" status=none
mkfs.ext4 -q -F -b 4096 -m 0 -E lazy_itable_init=0,lazy_journal_init=0 "$FIXTURE_ABS"

# Populate the image via debugfs (no mount privileges needed).
debugfs -w -R "mkdir /glibc" "$FIXTURE_ABS" >/dev/null
debugfs -w -R "mkdir /musl" "$FIXTURE_ABS" >/dev/null
debugfs -w -R "mkdir /work" "$FIXTURE_ABS" >/dev/null
debugfs -w -R "mkdir /work/tgoskits" "$FIXTURE_ABS" >/dev/null
debugfs -w -R "write $STAGING/SUDOOS_CONTEST_FIXTURE /SUDOOS_CONTEST_FIXTURE" "$FIXTURE_ABS" >/dev/null
debugfs -w -R "write $STAGING/arch /arch" "$FIXTURE_ABS" >/dev/null
debugfs -w -R "write $STAGING/glibc/cagent_testcode.sh /glibc/cagent_testcode.sh" "$FIXTURE_ABS" >/dev/null
debugfs -w -R "write $STAGING/musl/cagent_testcode.sh /musl/cagent_testcode.sh" "$FIXTURE_ABS" >/dev/null
debugfs -w -R "write $STAGING/work/tgoskits/Cargo.toml /work/tgoskits/Cargo.toml" "$FIXTURE_ABS" >/dev/null

echo "fixture: $FIXTURE_ABS (${FIXTURE_MIB} MiB, arch=$ARCH)"
