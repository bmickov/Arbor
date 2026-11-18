#!/bin/sh
# Sets the <release> line of Arbor's AppStream metadata to the current
# version (from src-tauri/Cargo.toml) and the date of the commit being built,
# so software centres show the right version and "last updated" date.
# CI runs it before packaging; run it yourself after bumping the version.
set -eu
cd "$(dirname "$0")/.."
file=src-tauri/linux/app.arbor.git.metainfo.xml
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' src-tauri/Cargo.toml | head -n 1)
date=$(git log -1 --format=%cs 2>/dev/null || date -u +%Y-%m-%d)
sed "s|<release version=\"[^\"]*\" date=\"[^\"]*\"/>|<release version=\"$version\" date=\"$date\"/>|" "$file" > "$file.tmp"
mv "$file.tmp" "$file"
grep -q "<release version=\"$version\" date=\"$date\"/>" "$file" || { echo "Couldn't update the release in $file" >&2; exit 1; }
echo "$file: release $version ($date)"
