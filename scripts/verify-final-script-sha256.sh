#!/bin/sh
set -eu

# Source: https://github.com/oscomp/testsuits-for-oskernel/blob/d69becb811573aa789a788e2940fa5ed8f9388f3/scripts/buildstorm_testcode.sh

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
manifest="$root/scripts/final-buildstorm-official.sha256"

if command -v sha256sum >/dev/null 2>&1; then
    (cd "$root" && sha256sum -c "$manifest")
elif command -v shasum >/dev/null 2>&1; then
    expected=$(awk '{print $1}' "$manifest")
    path=$(awk '{print $2}' "$manifest")
    actual=$(shasum -a 256 "$root/$path" | awk '{print $1}')
    test "$actual" = "$expected" || {
        echo "official BuildStorm script SHA256 mismatch" >&2
        exit 1
    }
    echo "$path: OK"
else
    echo "no SHA256 implementation found" >&2
    exit 1
fi
