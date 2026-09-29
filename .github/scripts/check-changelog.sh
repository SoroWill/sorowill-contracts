#!/bin/bash
set -euo pipefail

BASE_REF="origin/${GITHUB_BASE_REF:-main}"

if ! git rev-parse --verify --quiet "$BASE_REF" >/dev/null; then
    echo "::error::Cannot resolve base ref $BASE_REF; checkout must fetch full history (fetch-depth: 0)"
    exit 1
fi

if git diff --name-only "$BASE_REF" | grep -q 'contracts/will/src/lib.rs'; then
    if git diff "$BASE_REF" -- contracts/will/src/lib.rs | grep -q 'CONTRACT_VERSION'; then
        if ! git diff --name-only "$BASE_REF" | grep -q 'CHANGELOG.md'; then
            echo "::error::CHANGELOG.md must be updated when CONTRACT_VERSION is changed in contracts/will/src/lib.rs"
            exit 1
        fi
    fi
fi
