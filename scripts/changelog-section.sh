#!/bin/bash
# Prints the body of one CHANGELOG.md section: changelog-section.sh <heading-name> (a version or "Unreleased").
# Fails when the section is missing or empty.
set -e

NAME=$1
FILE=${2:-CHANGELOG.md}
if [[ -z "$NAME" ]]; then
  echo "Usage: changelog-section.sh <version|Unreleased> [file]" >&2
  exit 1
fi

BODY=$(awk -v name="$NAME" '
  /^## / {
    if (found) exit
    title = substr($0, 4)
    sub(/ - .*$/, "", title)
    if (title == name) found = 1
    next
  }
  found { print }
' "$FILE")

if [[ -z "$(echo "$BODY" | tr -d '[:space:]')" ]]; then
  echo "Error: no entries under '## $NAME' in $FILE" >&2
  exit 1
fi

echo "$BODY" | sed -e :a -e '/^[[:space:]]*$/{$d;N;ba' -e '}' | sed '/./,$!d'
