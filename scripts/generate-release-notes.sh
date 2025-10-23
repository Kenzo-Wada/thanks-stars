#!/usr/bin/env bash
set -euo pipefail

: "${ANNOUNCEMENT_BODY:?ANNOUNCEMENT_BODY is required}"
: "${RELEASE_TAG:?RELEASE_TAG is required}"
: "${REPOSITORY:?REPOSITORY is required}"
: "${RUNNER_TEMP:?RUNNER_TEMP is required}"

body="$ANNOUNCEMENT_BODY"
tag="$RELEASE_TAG"
repo="$REPOSITORY"

# Ensure installer URLs point at the latest release to match README guidance.
body=$(printf '%s' "$body" |
  perl -0pe "s{https://github.com/\Q${repo}\E/releases/download/\Q${tag}\E/(thanks-stars-installer\\.(?:sh|ps1))}{https://github.com/${repo}/releases/latest/download/\$1}g")

# Normalize installer snippets to match the README guidance.
body=$(printf '%s' "$body" | perl -0pe "s{curl --proto '=https' --tlsv1\.2 -LsSf}{curl -LSfs}g")
body=$(printf '%s' "$body" |
  perl -0pe 's{powershell -ExecutionPolicy Bypass -c "irm ([^"]+) \| iex"}{iwr $1 -useb | iex}g')
body=$(printf '%s' "$body" |
  perl -0pe 's{brew install Kenzo-Wada/thanks-stars/thanks-stars}{brew tap Kenzo-Wada/thanks-stars\nbrew install thanks-stars}g')

version="${tag#v}"

tag_present=true
if ! git rev-parse -q --verify "${tag}^{commit}" >/dev/null 2>&1; then
  tag_present=false
fi

# Extract the changelog section for the current version if available.
changelog=""
if [[ -f CHANGELOG.md ]]; then
  changelog=$(VERSION="$version" perl -0ne '
    my $version = $ENV{VERSION};
    if (/^##\s+\[?\Q$version\E(?:\s+-\s+[^\n]+)?\n(.*?)(?=^##\s+\[?.+\]?|\z)/ms) {
      print $1;
    }
  ' CHANGELOG.md || true)
  changelog=$(printf '%s' "$changelog" | sed 's/[[:space:]]*$//' )
fi
if [[ -z "${changelog//[[:space:]]/}" ]]; then
  changelog="_No changelog entries for this release._"
fi

# Determine the previous tag for contributor detection.
prev_tag=""
range_spec=""
if $tag_present; then
  git fetch --tags --force >/dev/null 2>&1 || true
  mapfile -t tags < <(git tag --sort=-version:refname)
  for idx in "${!tags[@]}"; do
    if [[ "${tags[$idx]}" == "$tag" ]]; then
      next_idx=$((idx + 1))
      if (( next_idx < ${#tags[@]} )); then
        prev_tag="${tags[$next_idx]}"
      fi
      break
    fi
  done
  if [[ -z "$prev_tag" && ${#tags[@]} -gt 1 ]]; then
    prev_tag="${tags[1]}"
  fi
  if [[ -n "$prev_tag" ]]; then
    range_spec="${prev_tag}..${tag}"
  else
    range_spec="$tag"
  fi
else
  range_spec="HEAD"
fi

mapfile -t contributor_entries < <(git log "$range_spec" --format='%aN|%aE' 2>/dev/null | sed '/^$/d' | sort -u)
if (( ${#contributor_entries[@]} )); then
  contributor_section=""
  declare -A seen_contributors=()
  declare -A seen_names=()
  for entry in "${contributor_entries[@]}"; do
    name="${entry%%|*}"
    email="${entry#*|}"
    username=""
    if [[ "$email" =~ ^[0-9]+\+([A-Za-z0-9-]+)@users\.noreply\.github\.com$ ]]; then
      username="${BASH_REMATCH[1]}"
    elif [[ "$email" =~ ^([A-Za-z0-9-]+)@users\.noreply\.github\.com$ ]]; then
      username="${BASH_REMATCH[1]}"
    fi

    if [[ -n "$username" ]]; then
      key="user:${username}"
      link="https://github.com/${username}"
      display="@${username}"
      normalized=$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]' | tr -cd '[:alnum:]')
      if [[ -n "$normalized" ]]; then
        seen_names[$normalized]=1
      fi
    else
      normalized=$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]' | tr -cd '[:alnum:]')
      if [[ -n "$normalized" && -n "${seen_names[$normalized]:-}" ]]; then
        continue
      fi
      key="name:${normalized:-$name}"
    fi

    if [[ -n "${seen_contributors[$key]:-}" ]]; then
      continue
    fi
    seen_contributors[$key]=1

    if [[ -n "$username" ]]; then
      contributor_section+="- [${display}](${link})"
      if [[ "$name" != "$display" ]]; then
        contributor_section+=" (${name})"
      fi
    else
      if command -v jq >/dev/null 2>&1; then
        query=$(printf 'repo:%s author:"%s"' "$repo" "$name" | jq -sRr @uri)
        search_link="https://github.com/search?q=${query}&type=commits"
        contributor_section+="- [${name}](${search_link})"
      else
        contributor_section+="- ${name}"
      fi
      if [[ -n "$normalized" ]]; then
        seen_names[$normalized]=1
      fi
    fi
    contributor_section+=$'\n'
  done
  contributor_section=$(printf '%s' "$contributor_section" | sed 's/[[:space:]]*$//')
else
  contributor_section="_No new contributors in this release._"
fi

if $tag_present; then
  if [[ -n "$prev_tag" ]]; then
    changes=$(git-cliff --config cliff.toml --range "${prev_tag}..${tag}" --strip footer || true)
  else
    changes=$(git-cliff --config cliff.toml --tag "$tag" --strip footer || true)
  fi
else
  changes=$(git-cliff --config cliff.toml --strip footer || true)
fi
changes=$(printf '%s' "$changes" |
  perl -0pe 's{\(commit: ([0-9a-fA-F]{7,40})\)}{my $sha = $1; my $short = substr($sha, 0, 7); "([${short}](https://github.com/'"$repo"'/commit/${sha}))"}ge')
changes=$(printf '%s' "$changes" | sed 's/[[:space:]]*$//')

sections=()
sections+=("$body")
if [[ -n "${changes//[[:space:]]/}" ]]; then
  sections+=("$changes")
fi
sections+=("## Changelog" "$changelog" "## Contributors" "$contributor_section")

notes=""
for section in "${sections[@]}"; do
  trimmed=$(printf '%s' "$section" | sed 's/[[:space:]]*$//')
  if [[ -n "${trimmed//[[:space:]]/}" ]]; then
    if [[ -n "$notes" ]]; then
      notes+=$'\n\n'
    fi
    notes+="$trimmed"
  fi
done
notes+=$'\n'

output_path="${RUNNER_TEMP}/notes.md"
printf '%s' "$notes" > "$output_path"

echo "path=$output_path" >> "$GITHUB_OUTPUT"
