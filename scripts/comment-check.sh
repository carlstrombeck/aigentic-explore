#!/usr/bin/env bash
# Judges the comments a diff adds to Rust files against AGENTS.md's "Comments"
# section and prints one line per offending comment. Comments that already
# exist are never judged, so the log's existing debt stays untouched.
set -euo pipefail

readonly USAGE='usage: scripts/comment-check.sh <base>'
readonly MARKER='// comment-check: allow'
readonly HUNK='^@@ -[0-9]+(,[0-9]+)? \+([0-9]+)(,[0-9]+)? @@'
readonly NUMBER='#[0-9]+'
readonly LABEL_START='^T[0-9]+([^0-9]|$)'
readonly LABEL_INSIDE='(^|[^[:alnum:]_])T[0-9]+ \('
PHRASES=('as before' 'used to' 'now that' "HEAD's")

# A hit is exit 1 and a clean run 0, so the only remaining code, 2, means the
# check could not do its job: a base that does not resolve never reads as a pass.
fail() {
  printf 'comment-check: %s\n' "$1" >&2
  exit 2
}

# Print the rule names a comment body breaks, joined with ", " in the order
# the rules are listed in AGENTS.md, and nothing when it breaks none.
rules_for() {
  local body=$1 rules=""
  local phrase

  if [[ $body =~ $NUMBER ]]; then
    rules='issue or PR number'
  fi

  shopt -s nocasematch
  if [[ $body == *'issue #'* ]] && [ -z "$rules" ]; then
    rules='issue reference'
  fi
  shopt -u nocasematch

  if [[ $body =~ $LABEL_START ]] || [[ $body =~ $LABEL_INSIDE ]]; then
    rules="${rules:+$rules, }test-plan label"
  fi

  for phrase in "${PHRASES[@]}"; do
    shopt -s nocasematch
    if [[ $body == *"$phrase"* ]]; then
      rules="${rules:+$rules, }history phrase \"$phrase\""
    fi
    shopt -u nocasematch
  done

  printf '%s' "$rules"
}

# Judge one added line. A line whose first non-blank characters are not a `//`
# comment is not judged at all, and neither is one cleared by the marker at the
# end. Sets the counters and prints the hit.
check_added() {
  local path=$1 lineno=$2 raw=$3
  local line body rules

  line=${raw%"${raw##*[![:space:]]}"}
  if [[ $line == *"$MARKER" ]]; then
    return
  fi
  line=${line#"${line%%[![:space:]]*}"}
  if [[ $line != //* ]]; then
    return
  fi

  # The body is the text after the doc marker: the run of slashes, the `!` of
  # an inner doc comment, then the spaces that follow.
  body=$line
  while [[ $body == /* ]]; do
    body=${body#/}
  done
  body=${body#"!"}
  body=${body#"${body%%[![:space:]]*}"}

  checked=$((checked + 1))
  rules=$(rules_for "$body")
  if [ -z "$rules" ]; then
    return
  fi
  hits=$((hits + 1))
  printf '%s:%s: %s: %s\n' "$path" "$lineno" "$rules" "$line"
}

if [ "$#" -ne 1 ]; then
  fail "$USAGE"
fi

top=$(git rev-parse --show-toplevel 2>/dev/null) ||
  fail 'not inside a git work tree'
base_commit=$(git -C "$top" rev-parse --verify --quiet "$1^{commit}") ||
  fail "the base '$1' does not resolve to a commit"
merge_base=$(git -C "$top" merge-base "$base_commit" HEAD) ||
  fail "no merge base between $1 and HEAD"

# A pager would wait, a configured textconv or ext-diff would rewrite the very
# lines being judged, and a rename would otherwise count every line as added.
diff=$(git -C "$top" --no-pager -c core.quotePath=false diff \
  --unified=0 --no-color --no-ext-diff --no-textconv --find-renames \
  "$merge_base" HEAD -- '*.rs') || fail 'git diff failed'

hits=0
checked=0
path=''
lineno=0
in_header=0
in_hunk=0

while IFS= read -r raw; do
  if [[ $raw == 'diff --git '* ]]; then
    in_header=1
    in_hunk=0
    path=''
    continue
  fi
  if [ "$in_header" -eq 1 ]; then
    if [[ $raw == '+++ '* ]]; then
      path=${raw#+++ }
      path=${path#b/}
      in_header=0
    fi
    continue
  fi
  if [[ $raw =~ $HUNK ]]; then
    lineno=${BASH_REMATCH[2]}
    in_hunk=1
    continue
  fi
  if [ "$in_hunk" -eq 0 ]; then
    continue
  fi

  case $raw in
    +*)
      check_added "$path" "$lineno" "${raw#+}"
      lineno=$((lineno + 1))
      ;;
    -*) ;;
    '\'*) ;;
    *)
      lineno=$((lineno + 1))
      ;;
  esac
done <<<"$diff"

printf 'comment-check: %s hits in %s added comment lines\n' "$hits" "$checked" >&2
if [ "$hits" -gt 0 ]; then
  exit 1
fi
exit 0
