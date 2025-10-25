#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 <major|minor|patch>" >&2
  exit 1
}

if [[ $# -ne 1 ]]; then
  usage
fi

bump="$1"
case "$bump" in
  major|minor|patch) ;;
  *) usage ;;
esac

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

cargo set-version --bump "$bump"

metadata="$(cargo metadata --no-deps --format-version 1)"
package="$(printf '%s\n' "$metadata" | awk -F'"' '
  $2 == "packages" { in_packages=1; next }
  in_packages && $2 == "name" { print $4; exit }
')"
version="$(printf '%s\n' "$metadata" | awk -F'"' '
  $2 == "packages" { in_packages=1; next }
  in_packages && $2 == "version" { print $4; exit }
')"

cargo update --package "$package" --precise "$version"

git commit -am "chore(release): v$version"
git tag "v$version"
git push origin HEAD
git push origin "v$version"
