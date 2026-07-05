#!/bin/bash
set -e

CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "$CURRENT_BRANCH" != "main" ]]; then
  echo "Error: You must be on the main branch (currently on '$CURRENT_BRANCH')"
  exit 1
fi

git pull --ff-only
VERSION=$(grep '^version' crates/sim/Cargo.toml | head -n1 | sed -E 's/version *= *"([^"]+)"/\1/')
git add Cargo.lock crates/sim/Cargo.toml crates/rngo/Cargo.toml crates/cli/Cargo.toml
git commit -m "$VERSION"
git tag $VERSION
git push origin main --tags
