# aigentic-policy

Permission rules for tool calls. Phase 3 of `docs/PLAN-phase3.md`;
depends on `aigentic-core` and, for the permission request,
`aigentic-api`.

`Policy::decide(call, class)` evaluates `rules` first match wins and
returns `Allow { rule }`, `Ask { reason }` or `Deny { rule }`. The rule
name (`class read`, `tool mcp.*`, `bash allow-pattern cargo test`) is
what the runtime records on the tool result. No matching rule asks.

`Policy::defaults()` is the plan's table: `safe` and `read` allow, a
`bash` command matching an allow pattern allows, `exec`, `write` and
`network` ask, `mcp.*` asks. A command line is read segment by
segment (#15): the scanner reads quoting, so `grep -n "a|b"` is one
command and not a pipe, and splits at `|`, `||`, `&&`, `&`, `;`,
newlines and grouping parens. A line runs without asking when every
segment is an allow-list command without its writing flags (`sed -i`,
`find -delete`, `git branch -d`, `sort -o`, `git diff --output`,
`git log --output`, `git show --output`, `git diff --textconv`,
`git log --textconv`, `git show --textconv`, `git grep -O`,
`git grep --textconv`, an
awk program with `system(` or a redirection, `gh api` without GET),
`cd`, an assignment, or a substitution that is itself read-only —
inside a `git log` or `git show` argument, one that can print only a
repository token.
`2>&1`, `>/dev/null` and `2>/dev/null` are harmless; any other `>` or
`>>` writes. `Policy::configured` maps `[policy]` in `aigentic.toml`:
`rules` are prepended, `bash_allow` replaces the defaults.

A repository's own git config is trusted on the default path: plain
`git diff`, `git log -p`, `git show` and their `--stat` and `--oneline`
forms keep running without asking when the repository names a textconv
or an external diff driver, which is what shows a binary file as the
text the driver prints. `.gitattributes` alone names no program, and a
clone does not bring `.git/config`, so `diff.<driver>.textconv`,
`diff.<driver>.command` and `diff.external` can only come from local or
global config — and a model writes one of those only through an edit or
write tool, which already asks. The command line naming the filter
itself still asks: `--textconv`, `-a`/`--text`, and `--output`. The
classifier reads the command line, never repository state, so a
configured driver changes what git runs and not what policy says.
