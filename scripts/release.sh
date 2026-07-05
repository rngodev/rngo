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

CURRENT=$(grep '^version' crates/sim/Cargo.toml | head -n1 | sed -E 's/version *= *"([^"]+)"/\1/')
MAJOR=$(echo $CURRENT | cut -d. -f1)
MINOR=$(echo $CURRENT | cut -d. -f2)
PATCH=$(echo $CURRENT | cut -d. -f3)

if [[ "$BUMP" == "minor" ]]; then
  VERSION="$MAJOR.$((MINOR + 1)).0"
else
  VERSION="$MAJOR.$MINOR.$((PATCH + 1))"
fi

echo "Releasing $CURRENT -> $VERSION"

sed -i.bak -E "s/^version = \"[^\"]+\"/version = \"$VERSION\"/" crates/sim/Cargo.toml
sed -i.bak -E "s/^version = \"[^\"]+\"/version = \"$VERSION\"/" crates/rngo/Cargo.toml
sed -i.bak -E "s/^rngo-sim = \{ version = \"[^\"]+\"/rngo-sim = { version = \"$VERSION\"/" crates/rngo/Cargo.toml
sed -i.bak -E "s/^version = \"[^\"]+\"/version = \"$VERSION\"/" crates/cli/Cargo.toml
find crates -name "*.bak" -delete

cargo generate-lockfile

git add Cargo.lock crates/sim/Cargo.toml crates/rngo/Cargo.toml crates/cli/Cargo.toml
git commit -m "$VERSION"
git tag $VERSION
git push origin main --tags
