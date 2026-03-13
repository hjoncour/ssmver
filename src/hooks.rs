pub const PREPARE_COMMIT_MSG: &str = r#"#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname "$0")/../scripts" && pwd)
. "$SCRIPT_DIR/bump_version.sh"

ssmver_prepare_commit_msg "$1" "${2:-}"
"#;

pub const POST_COMMIT: &str = r#"#!/bin/sh
set -eu

if [ "${SSMVER_AMENDING:-}" = "1" ]; then
  exit 0
fi

if [ ! -f "ssmver.toml" ]; then
  exit 0
fi

if git diff --quiet -- ssmver.toml; then
  exit 0
fi

git add ssmver.toml
SSMVER_AMENDING=1 git commit --amend --no-edit >/dev/null
"#;

pub const BUMP_VERSION: &str = r#"#!/bin/sh
set -eu

SSMVER_CONFIG="ssmver.toml"

ssmver_get_version() {
  sed -n 's/^version[[:space:]]*=[[:space:]]*"\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' "$SSMVER_CONFIG" | head -n 1
}

ssmver_version_from_content() {
  printf '%s\n' "$1" | sed -n 's/^version[[:space:]]*=[[:space:]]*"\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' | head -n 1
}

ssmver_get_table_value() {
  table="$1"
  key="$2"

  awk -v table="$table" -v key="$key" '
    BEGIN { in_table = 0 }
    /^[[:space:]]*\[/ {
      in_table = ($0 == "[" table "]")
      next
    }
    in_table {
      line = $0
      sub(/[[:space:]]*#.*/, "", line)
      if (line ~ /^[[:space:]]*$/) {
        next
      }

      split(line, parts, "=")
      current_key = parts[1]
      gsub(/[[:space:]]/, "", current_key)
      if (current_key != key) {
        next
      }

      value = substr(line, index(line, "=") + 1)
      sub(/^[[:space:]]*/, "", value)
      sub(/[[:space:]]*$/, "", value)
      sub(/^"/, "", value)
      sub(/"$/, "", value)
      print value
      exit
    }
  ' "$SSMVER_CONFIG"
}

ssmver_get_setting() {
  ssmver_get_table_value "settings" "$1"
}

ssmver_get_prefix_level() {
  ssmver_get_table_value "prefixes" "$1"
}

ssmver_extract_prefix() {
  printf '%s\n' "$1" | sed -n 's/^\([[:alnum:]_-][[:alnum:]_-]*\)\(([^)]*)\)\{0,1\}:.*/\1/p'
}

ssmver_level_priority() {
  case "$1" in
    patch) printf '1\n' ;;
    minor) printf '2\n' ;;
    major) printf '3\n' ;;
    *) printf '0\n' ;;
  esac
}

ssmver_bump_version() {
  version="$1"
  level="$2"

  old_ifs=$IFS
  IFS=.
  set -- $version
  IFS=$old_ifs

  major="$1"
  minor="$2"
  patch="$3"

  case "$level" in
    patch)
      patch=$((patch + 1))
      ;;
    minor)
      minor=$((minor + 1))
      patch=0
      ;;
    major)
      major=$((major + 1))
      minor=0
      patch=0
      ;;
    *)
      return 1
      ;;
  esac

  printf '%s.%s.%s\n' "$major" "$minor" "$patch"
}

ssmver_write_version() {
  new_version="$1"
  tmp_file="${SSMVER_CONFIG}.tmp.$$"

  awk -v new_version="$new_version" '
    BEGIN { updated = 0 }
    !updated && /^version[[:space:]]*=/ {
      print "version = \"" new_version "\""
      updated = 1
      next
    }
    { print }
    END {
      if (!updated) {
        exit 1
      }
    }
  ' "$SSMVER_CONFIG" > "$tmp_file"

  mv "$tmp_file" "$SSMVER_CONFIG"
}

ssmver_find_main_ref() {
  for ref in refs/heads/main refs/heads/master refs/remotes/origin/main refs/remotes/origin/master
  do
    if git show-ref --verify --quiet "$ref"; then
      printf '%s\n' "$ref"
      return 0
    fi
  done

  return 1
}

ssmver_highest_level_from_file() {
  file="$1"
  highest=""

  while IFS= read -r subject
  do
    prefix=$(ssmver_extract_prefix "$subject" || true)
    [ -n "$prefix" ] || continue

    level=$(ssmver_get_prefix_level "$prefix" || true)
    [ -n "$level" ] || continue

    if [ -z "$highest" ] || [ "$(ssmver_level_priority "$level")" -gt "$(ssmver_level_priority "$highest")" ]; then
      highest="$level"
    fi
  done < "$file"

  printf '%s\n' "$highest"
}

ssmver_branch_version() {
  new_level="$1"
  current_version=$(ssmver_get_version)
  main_ref=$(ssmver_find_main_ref || true)

  if [ -z "$main_ref" ]; then
    ssmver_bump_version "$current_version" "$new_level"
    return 0
  fi

  merge_base=$(git merge-base HEAD "$main_ref" 2>/dev/null || true)
  if [ -z "$merge_base" ]; then
    ssmver_bump_version "$current_version" "$new_level"
    return 0
  fi

  subjects_file=$(mktemp)
  git log --format=%s "$merge_base..HEAD" > "$subjects_file" 2>/dev/null || true
  previous_highest=$(ssmver_highest_level_from_file "$subjects_file")
  rm -f "$subjects_file"

  if [ -n "$previous_highest" ] && [ "$(ssmver_level_priority "$new_level")" -le "$(ssmver_level_priority "$previous_highest")" ]; then
    return 1
  fi

  base_content=$(git show "$merge_base:$SSMVER_CONFIG" 2>/dev/null || true)
  base_version=$(ssmver_version_from_content "$base_content")
  if [ -z "$base_version" ]; then
    base_version="$current_version"
  fi

  ssmver_bump_version "$base_version" "$new_level"
}

ssmver_needs_prompt() {
  prefix="$1"
  prompt_mode=$(ssmver_get_setting "prompt" || true)

  case "$prompt_mode" in
    always)
      return 0
      ;;
    ask)
      raw=$(ssmver_get_setting "prompt_prefixes" || true)
      parsed=$(printf '%s\n' "$raw" | tr ',' '\n' | sed 's/\[//g; s/\]//g; s/"//g; s/^[[:space:]]*//; s/[[:space:]]*$//')
      while IFS= read -r candidate
      do
        [ -n "$candidate" ] || continue
        if [ "$candidate" = "$prefix" ]; then
          return 0
        fi
      done <<EOF
$parsed
EOF
      return 1
      ;;
    *)
      return 1
      ;;
  esac
}

ssmver_commit_has_body() {
  message_file="$1"
  sed -n '2,$p' "$message_file" | grep -q '[^[:space:]]'
}

ssmver_prompt_for_body() {
  message_file="$1"
  prefix="$2"

  if ssmver_commit_has_body "$message_file"; then
    return 0
  fi

  if [ ! -r /dev/tty ]; then
    return 0
  fi

  printf 'ssmver description for %s commit (optional): ' "$prefix" > /dev/tty
  if ! IFS= read -r body < /dev/tty; then
    return 0
  fi

  if [ -n "$body" ]; then
    printf '\n%s\n' "$body" >> "$message_file"
  fi
}

ssmver_prepare_commit_msg() {
  message_file="$1"
  source="${2:-}"

  if [ "${SSMVER_AMENDING:-}" = "1" ]; then
    exit 0
  fi

  case "$source" in
    merge|squash|commit)
      exit 0
      ;;
  esac

  if [ ! -f "$SSMVER_CONFIG" ]; then
    exit 0
  fi

  first_line=$(sed -n '1p' "$message_file")
  prefix=$(ssmver_extract_prefix "$first_line" || true)
  [ -n "$prefix" ] || exit 0

  level=$(ssmver_get_prefix_level "$prefix" || true)
  [ -n "$level" ] || exit 0

  mode=$(ssmver_get_setting "mode" || true)
  current_version=$(ssmver_get_version)

  case "$mode" in
    branch)
      new_version=$(ssmver_branch_version "$level") || exit 0
      ;;
    *)
      new_version=$(ssmver_bump_version "$current_version" "$level")
      ;;
  esac

  if [ "$new_version" != "$current_version" ]; then
    ssmver_write_version "$new_version"
  fi

  if ssmver_needs_prompt "$prefix"; then
    ssmver_prompt_for_body "$message_file" "$prefix"
  fi
}
"#;
