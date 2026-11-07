#!/usr/bin/env bash
# Prints every workspace's projects with each brief's first line and its
# `## Current state` section: the by-hand status view of
# docs/PLAN-portfolio.md slice 1. Reads the briefs on disk directly,
# because a thread may only read briefs in its own workspace.
set -euo pipefail

readonly AIGENTIC=${AIGENTIC:-aigentic}
# Under each workspace the first row is its `shared` folder, then one row
# per project: its name, two spaces, its absolute path. A name may hold
# spaces, so the path is found by its leading `/`.
readonly PROJECT_ROW='^  (.*[^ ])  (/.*)$'

# The first non-empty line, then the Current state body up to the next
# `## ` heading. Fenced code is not special-cased: a brief is prose.
summarise() {
  awk '
    !first && NF { print "    " $0; first = 1; next }
    tolower($0) ~ /^## current state[[:space:]]*$/ { inside = 1; next }
    inside && /^## / { exit }
    inside && NF { print "      " $0 }
  ' "$1"
}

"$AIGENTIC" workspace list | while IFS= read -r line; do
  if [[ $line != " "* ]]; then
    printf '\n== %s\n' "$line"
  elif [[ $line =~ $PROJECT_ROW && ${BASH_REMATCH[1]} != shared ]]; then
    name=${BASH_REMATCH[1]}
    brief=${BASH_REMATCH[2]}/.aigentic/brief.md
    printf '  %s\n' "$name"
    if [[ -f $brief ]]; then
      summarise "$brief"
    else
      printf '    (no brief)\n'
    fi
  fi
done
