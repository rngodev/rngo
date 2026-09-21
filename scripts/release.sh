#!/bin/bash
set -e

BUMP=$1
if [[ "$BUMP" != "minor" && "$BUMP" != "patch" ]]; then
  echo "Error: argument must be 'minor' or 'patch'"
  exit 1
fi

CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "$CURRENT_BRANCH" != "main" ]]; then
  echo "Error: You must be on the main branch (currently on '$CURRENT_BRANCH')"
  exit 1
fi

git pull --ff-only

CURRENT=$(grep '^version' crates/core/Cargo.toml | head -n1 | sed -E 's/version *= *"([^"]+)"/\1/')
MAJOR=$(echo $CURRENT | cut -d. -f1)
MINOR=$(echo $CURRENT | cut -d. -f2)
PATCH=$(echo $CURRENT | cut -d. -f3)

if [[ "$BUMP" == "minor" ]]; then
  VERSION="$MAJOR.$((MINOR + 1)).0"
else
  VERSION="$MAJOR.$MINOR.$((PATCH + 1))"
fi

echo "Releasing $CURRENT -> $VERSION"

# Every crate in the workspace is versioned in lockstep, and every internal
# `rngo-* = { version = "...", path = "..." }` dependency between them is bumped
# alongside it.
CRATES=(core log effect proxy audit rngo cli)

for crate in "${CRATES[@]}"; do
  sed -i.bak -E "s/^version = \"[^\"]+\"/version = \"$VERSION\"/" "crates/$crate/Cargo.toml"
  sed -i.bak -E "s/^(rngo-[a-z]+ = \{ version = \")[^\"]+/\1$VERSION/" "crates/$crate/Cargo.toml"
done
find crates -name "*.bak" -delete

cargo generate-lockfile

git add Cargo.lock
for crate in "${CRATES[@]}"; do
  git add "crates/$crate/Cargo.toml"
done
git commit -m "$VERSION"
git tag $VERSION
git push origin main --tags
