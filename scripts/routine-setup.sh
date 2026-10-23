#!/usr/bin/env bash
# Sets up the by-hand morning routine of docs/PLAN-portfolio.md slice 1:
# each project joins its workspace and gets a brief to fill in, and a
# portfolio project holds the routine's log. Idempotent: a file that
# exists is never overwritten, and a project already in its workspace is
# left as it is.
set -euo pipefail

readonly USAGE='usage: scripts/routine-setup.sh <portfolio-dir> <workspace>=<project-dir>...'
readonly AIGENTIC=${AIGENTIC:-aigentic}
readonly TEMPLATES="$(cd "$(dirname "${BASH_SOURCE[0]}")/../docs/routine" && pwd)"
readonly PORTFOLIO_WORKSPACE=portfolio

fail() {
  printf 'routine-setup: %s\n' "$1" >&2
  exit 2
}

# `workspace new` refuses a name that exists, so ask the list first.
ensure_workspace() {
  local name=$1
  if ! "$AIGENTIC" workspace list | grep -qx -- "$name"; then
    "$AIGENTIC" workspace new "$name"
  fi
}

# A folder with no aigentic.toml is initialised into the workspace; one
# that has it is listed in the workspace, which is a no-op when it is
# already there.
join_workspace() {
  local workspace=$1 dir=$2
  if [[ -f $dir/aigentic.toml ]]; then
    "$AIGENTIC" workspace add "$dir" --workspace "$workspace"
  else
    (cd "$dir" && "$AIGENTIC" project init --workspace "$workspace")
  fi
}

copy_if_absent() {
  local from=$1 to=$2 name=$3
  if [[ -e $to ]]; then
    printf 'kept %s\n' "$to"
  else
    mkdir -p "$(dirname "$to")"
    local text
    text=$(<"$from")
    printf '%s\n' "${text//PROJECT_NAME/"$name"}" >"$to"
    printf 'wrote %s\n' "$to"
  fi
}

(($# >= 2)) || fail "$USAGE"
command -v "$AIGENTIC" >/dev/null || fail "no $AIGENTIC on PATH (set AIGENTIC to the binary)"

portfolio=$1
shift

for pair in "$@"; do
  [[ $pair == *=* ]] || fail "not <workspace>=<project-dir>: $pair"
  workspace=${pair%%=*}
  dir=${pair#*=}
  [[ -n $workspace && -d $dir ]] || fail "no such project folder: $dir"
  [[ $workspace != "$PORTFOLIO_WORKSPACE" ]] || fail "the workspace name $PORTFOLIO_WORKSPACE is the portfolio's own"
  dir=$(cd "$dir" && pwd)
  ensure_workspace "$workspace"
  join_workspace "$workspace" "$dir"
  copy_if_absent "$TEMPLATES/brief.md" "$dir/.aigentic/brief.md" "$(basename "$dir")"
done

mkdir -p "$portfolio"
portfolio=$(cd "$portfolio" && pwd)
ensure_workspace "$PORTFOLIO_WORKSPACE"
join_workspace "$PORTFOLIO_WORKSPACE" "$portfolio"
copy_if_absent "$TEMPLATES/portfolio-instructions.md" "$portfolio/.aigentic/instructions.md" portfolio
copy_if_absent "$TEMPLATES/routine-log.md" "$portfolio/.aigentic/knowledge/routine-log.md" portfolio

cat <<NEXT

Next:
  1. Fill in each project's .aigentic/brief.md (first line, Foundation, Current state).
  2. Each morning: scripts/routine-briefs.sh, then aigentic status.
  3. Log the morning in $portfolio/.aigentic/knowledge/routine-log.md.
NEXT
