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

# Issue #439: CONTRACT_VERSION is compiled into the contract and is immutable
# after deployment. If the constant is bumped in source, the git version tag
# must be bumped in lockstep so SDKs can rely on the reported version to detect
# behavioral changes. Fail the merge when the constant changed but no version
# tag was added/updated in the same change set.
if git diff --name-only "$BASE_REF" | grep -q 'contracts/will/src/lib.rs'; then
    if git diff "$BASE_REF" -- contracts/will/src/lib.rs | grep -q 'CONTRACT_VERSION'; then
        if ! git diff "$BASE_REF" -- . | grep -qE '^\+.*(v[0-9]+\.[0-9]+\.[0-9]+|version[[:space:]]*=[[:space:]]*"[0-9]+\.[0-9]+\.[0-9]+")'; then
            echo "::error::CONTRACT_VERSION changed in contracts/will/src/lib.rs but no matching version tag/bump was found in the diff. Bump the git version tag (e.g. vX.Y.Z) alongside CONTRACT_VERSION."
            exit 1
        fi
    fi
fi
