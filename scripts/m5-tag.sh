#!/usr/bin/env bash
set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"
TAG="${M5_TAG:-m5-complete}"
LATEST="${ROOT_DIR}/build/m5/latest.txt"
[[ -f "${LATEST}" ]] || { echo "error: run make m5-release first" >&2; exit 1; }
REPORT="$(cat "${LATEST}")/report.json"
python3 - "${REPORT}" "$(git rev-parse HEAD)" <<'PY'
import json, sys
from pathlib import Path
r = json.loads(Path(sys.argv[1]).read_text())
assert r["status"] == "pass"
assert r["level"] == "soak"
assert r["git_head"] == sys.argv[2]
assert r["worktree_clean"] is True
PY
[[ -z "$(git status --porcelain)" ]] || { echo "error: dirty worktree" >&2; exit 1; }
git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null && { echo "error: tag exists" >&2; exit 1; }
git tag -a "${TAG}" -m "Complete M5 SMP concurrency foundation"
echo "created ${TAG}; push with: git push origin ${TAG}"
