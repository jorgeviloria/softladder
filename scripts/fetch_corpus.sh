#!/usr/bin/env bash
#
# Fetch the ClassicLadder compatibility corpus used by the M3 golden tests.
#
# Usage: scripts/fetch_corpus.sh [destination]
#
# The destination defaults to testdata/classicladder-corpus next to this
# repository and is excluded by .gitignore. The script is idempotent: when the
# corpus is already cloned it exits successfully without touching it.

set -euo pipefail

REPO_URL="https://github.com/MaVaTi56/classicladder.git"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${1:-${SCRIPT_DIR}/../testdata/classicladder-corpus}"

if [ -d "${DEST}/.git" ]; then
    echo "classicladder corpus already present at ${DEST}"
    exit 0
fi

if [ -e "${DEST}" ]; then
    echo "error: ${DEST} exists but is not a git checkout; remove it first" >&2
    exit 1
fi

mkdir -p "$(dirname "${DEST}")"
echo "cloning ${REPO_URL} into ${DEST} (shallow)"
git clone --depth 1 "${REPO_URL}" "${DEST}"

echo "classicladder corpus available at ${DEST}"
echo "example projects: $(find "${DEST}/projects_examples" -name '*.clprj*' | wc -l | tr -d ' ')"
