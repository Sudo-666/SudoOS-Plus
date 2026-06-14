#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

TAG="${M7_TAG:-m7-complete}"
LATEST="${ROOT_DIR}/build/m7/latest.txt"

[[ -f "${LATEST}" ]] || {
    echo "error: run make m7-release first" >&2
    exit 1
}

REPORT_DIR="$(cat "${LATEST}")"
REPORT="${REPORT_DIR}/report.json"

[[ -f "${REPORT}" ]] || {
    echo "error: M7 release report is missing: ${REPORT}" >&2
    exit 1
}

python3 - "${REPORT}" "$(git rev-parse HEAD)" <<'PY'
import json
import sys
from pathlib import Path

report = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
assert report["milestone"] == "M7-B", "not an M7-B report"
assert report["status"] == "pass", "latest M7 report did not pass"
assert report["level"] == "release", "latest M7 report is not release level"
assert report["git_head"] == sys.argv[2], "release report belongs to another commit"
assert report["worktree_clean"] is True, "release report was produced from a dirty tree"
PY

[[ -z "$(git status --porcelain=v1 --untracked-files=all)" ]] || {
    echo "error: worktree changed after M7 release verification" >&2
    exit 1
}

git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null && {
    echo "error: tag already exists: ${TAG}" >&2
    exit 1
}

git tag -a "${TAG}" -m \
    "Complete M7 minimal user mode, syscall, user-copy and fault-isolation runtime"

echo "created ${TAG}"
echo "push commit and tag with: git push origin HEAD:main && git push origin ${TAG}"
