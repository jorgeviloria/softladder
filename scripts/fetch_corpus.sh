#!/usr/bin/env bash
#
# Fetch the ClassicLadder compatibility corpus used by the M3 golden tests.
#
# Usage: scripts/fetch_corpus.sh [destination]
#
# The destination defaults to testdata/classicladder-corpus next to this
# repository and is excluded by .gitignore. It ends up holding the upstream
# `projects_examples/` directory, i.e. `<destination>/projects_examples/*.clprj*`.
#
# The script is idempotent and works offline when a ClassicLadder checkout is
# available next to this repository: in that case the example projects are
# copied instead of cloned. Set CORPUS_LOCAL_CHECKOUT to point at a different
# checkout, or CORPUS_FORCE_CLONE=1 to always clone.

set -euo pipefail

REPO_URL="https://github.com/MaVaTi56/classicladder.git"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${1:-${SCRIPT_DIR}/../testdata/classicladder-corpus}"
LOCAL_CHECKOUT="${CORPUS_LOCAL_CHECKOUT:-${SCRIPT_DIR}/../../classicladder}"

if [ -d "${DEST}/projects_examples" ]; then
    echo "corpus already present at ${DEST}"
    echo "example projects: $(find "${DEST}/projects_examples" -name '*.clprj*' -o -name '*.clp' | wc -l | tr -d ' ')"
    exit 0
fi

if [ -e "${DEST}" ]; then
    echo "error: ${DEST} exists but holds no projects_examples/; remove it first" >&2
    exit 1
fi

if [ -z "${CORPUS_FORCE_CLONE:-}" ] && [ -d "${LOCAL_CHECKOUT}/projects_examples" ]; then
    echo "copying example projects from the local checkout at ${LOCAL_CHECKOUT}"
    mkdir -p "${DEST}"
    cp -R "${LOCAL_CHECKOUT}/projects_examples" "${DEST}/projects_examples"
else
    echo "cloning ${REPO_URL} into ${DEST} (shallow)"
    git clone --depth 1 "${REPO_URL}" "${DEST}.tmp"
    mkdir -p "${DEST}"
    mv "${DEST}.tmp/projects_examples" "${DEST}/projects_examples"
    rm -rf "${DEST}.tmp"
fi

echo "corpus available at ${DEST}"
echo "example projects: $(find "${DEST}/projects_examples" -name '*.clprj*' -o -name '*.clp' | wc -l | tr -d ' ')"
