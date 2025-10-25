#!/usr/bin/env bash
# release.sh — bump version, sync Cargo.lock, commit, tag, and push.
# Usage: ./release.sh <major|minor|patch>
set -euo pipefail

usage() {
	echo "Usage: $0 <major|minor|patch>" >&2
	exit 1
}

#---------- Args ----------
if [[ $# -ne 1 ]]; then usage; fi
bump="$1"
case "$bump" in major | minor | patch) ;; *) usage ;; esac

#---------- Preflight ----------
command -v jq >/dev/null 2>&1 || {
	echo "Error: jq is required. Please install jq." >&2
	exit 1
}

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

# Ensure clean working tree (except untracked files)
if ! git diff --quiet || ! git diff --cached --quiet; then
	echo "Error: You have uncommitted changes. Commit or stash them first." >&2
	exit 1
fi

# Optional overrides:
#   PACKAGE:         package name to release (e.g., thanks-stars)
#   TARGET_MANIFEST: path to Cargo.toml to release (e.g., crates/thanks-stars/Cargo.toml)
PACKAGE="${PACKAGE:-}"
TARGET_MANIFEST="${TARGET_MANIFEST:-}"

#---------- Discover target package ----------
# Strategy:
# 1) If TARGET_MANIFEST is given, use that.
# 2) Else try repo_root/Cargo.toml (root package).
# 3) Else if the workspace has exactly one package, use that.
# If ambiguous, ask the user to set PACKAGE or TARGET_MANIFEST.

abs_manifest=""
if [[ -n "$TARGET_MANIFEST" ]]; then
	abs_manifest="$(
		python3 - <<PY
import os,sys
p=os.path.join("$repo_root", "$TARGET_MANIFEST")
print(os.path.abspath(p))
PY
	)"
	[[ -f "$abs_manifest" ]] || {
		echo "Error: TARGET_MANIFEST not found: $abs_manifest" >&2
		exit 1
	}
else
	# Default to root Cargo.toml if it defines a package
	if grep -qE '^\[package\]' "$repo_root/Cargo.toml" 2>/dev/null; then
		abs_manifest="$repo_root/Cargo.toml"
	fi
fi

metadata_json="$(cargo metadata --no-deps --format-version 1)"

pick_pkg_from_manifest() {
	local manifest="$1"
	jq -r --arg mf "$manifest" '
    .packages[] | select(.manifest_path == $mf) | "\(.name)\t\(.version)"
  ' <<<"$metadata_json"
}

pick_single_pkg_in_repo() {
	jq -r --arg repo "$repo_root/" '
    [ .packages[] | select(.manifest_path | startswith($repo)) ] as $pkgs
    | if ($pkgs | length) == 1
      then ($pkgs[0] | "\(.name)\t\(.version)")
      else empty
      end
  ' <<<"$metadata_json"
}

pair=""
if [[ -n "$abs_manifest" ]]; then
	pair="$(pick_pkg_from_manifest "$abs_manifest" || true)"
fi
if [[ -z "$pair" ]]; then
	pair="$(pick_single_pkg_in_repo || true)"
fi

pkg_name_from_meta="$(cut -f1 <<<"${pair:-}" || true)"
pkg_ver_from_meta="$(cut -f2 <<<"${pair:-}" || true)"

# If PACKAGE env is provided, prefer it (helps in multi-crate workspaces)
if [[ -n "$PACKAGE" ]]; then
	pkg_name="$PACKAGE"
else
	pkg_name="${pkg_name_from_meta:-}"
fi

if [[ -z "$pkg_name" ]]; then
	cat >&2 <<EOF
Error: Could not determine which package to release.

Hints:
  - Set PACKAGE env var:      PACKAGE=thanks-stars $0 $bump
  - Or set TARGET_MANIFEST:   TARGET_MANIFEST=crates/thanks-stars/Cargo.toml $0 $bump
EOF
	exit 1
fi

#---------- Bump version ----------
# Use -p to be explicit about the package (safe for workspaces too).
cargo set-version -p "$pkg_name" --bump "$bump"

# Read bumped version (re-query metadata)
metadata_json="$(cargo metadata --no-deps --format-version 1)"
new_pair="$(jq -r --arg name "$pkg_name" '
  .packages[] | select(.name == $name) | "\(.name)\t\(.version)"
' <<<"$metadata_json")"

if [[ -z "$new_pair" ]]; then
	echo "Error: Failed to read bumped version for package '$pkg_name'." >&2
	exit 1
fi

package="$(cut -f1 <<<"$new_pair")"
version="$(cut -f2 <<<"$new_pair")"

if [[ -z "$version" ]]; then
	echo "Error: version is empty after bump." >&2
	exit 1
fi

echo "Bumped $package to $version"

#---------- Sync npm package version (if present) ----------
pkg_json_path="$repo_root/package.json"
if [[ -f "$pkg_json_path" ]]; then
        if ! command -v node >/dev/null 2>&1; then
                echo "Error: Node.js is required to update the npm package version." >&2
                exit 1
        fi

        echo "Aligning npm package version to $version"
        RELEASE_VERSION="$version" PACKAGE_JSON_PATH="$pkg_json_path" node <<'NODE'
const fs = require('fs');
const path = require('path');

const pkgPath = process.env.PACKAGE_JSON_PATH || path.resolve(process.cwd(), 'package.json');
const releaseVersion = process.env.RELEASE_VERSION;

if (!releaseVersion) {
  console.error('Error: RELEASE_VERSION env var is not set.');
  process.exit(1);
}

const raw = fs.readFileSync(pkgPath, 'utf8');
const pkg = JSON.parse(raw);

pkg.version = releaseVersion;

const formatted = JSON.stringify(pkg, null, 2) + '\n';
fs.writeFileSync(pkgPath, formatted);
NODE
fi

#---------- Sync Cargo.lock ----------
# Keep lockfile pinned to the exact new version of the package (if present).
# Fallback to generate-lockfile if update fails (e.g., in minimal setups).
if ! cargo update -p "$package" --precise "$version"; then
	echo "cargo update failed; regenerating lockfile..."
	cargo generate-lockfile
fi

#---------- Commit & Tag ----------
# Only commit if something actually changed
if git diff --quiet && git diff --cached --quiet; then
	echo "No changes detected after bump; nothing to commit."
else
	git add -A
	git commit -m "chore(release): v$version"
fi

tag="v$version"
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
	echo "Tag $tag already exists. Skipping tag creation."
else
	git tag "$tag"
fi

#---------- Push ----------
git push origin HEAD
git push origin "$tag"

echo "Done: $package $version"
