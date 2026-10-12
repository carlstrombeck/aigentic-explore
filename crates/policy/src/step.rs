//! The per-step deny overlay.
//!
//! PLAN-layer2 §6.4 splits a task between a step thread and the runner:
//! the step writes and commits, the runner checks the tree, pushes once
//! and installs. A few commands and files belong to the runner alone, so
//! a step that runs them is a defect whatever the rules, the project or
//! the mode say. The runner supplies the list (`deny`), [`StepOverlay`]
//! parses it, and the runtime consults it once the rules have spoken —
//! its answer stands in theirs. See `Runtime::policy_check`.
//!
//! The grammar is small and closed on purpose: anything the overlay does
//! not recognise is a [`DenyParseError`] where the overlay is built, never
//! a silent no-op at call time.
//!
//! This is a lexical guard, not a sandbox (that is phase 7). A word
//! names a program by its final path component and quoting it changes
//! nothing, so `\git push` and `"git" push` deny as `git push`. Stated
//! limits: a command reached through `xargs`, `find -exec`, `eval` or
//! `python -c`; a wrapper with no row in
//! [`FORWARDERS`](crate::shell::FORWARDERS) (`doas`, `setsid`, `run0`,
//! `ionice`, `chrt`, `flock`, `unbuffer`, `script`) or a multi-call
//! binary that chooses its command from its own words (`busybox git
//! push`); `env -S <line>`, whose value is a command line the reader
//! does not split; a here-document body fed to a shell (`bash <<EOF`
//! with the command inside); `PATH` shadowing, where the reader reads
//! the word `git` and never the program it resolves to; and `git pull
//! --rebase`, which names no entry yet. A word that mixes literal text
//! with a substitution is refused rather than read: `$cmd push` reads as
//! `step deny unreadable`, and `cat $( git push )` as `step deny git
//! push`.

use std::fmt;

use aigentic_core::ToolCall;

use crate::Outcome;
use crate::shell::{SegRead, segment_reads};

/// The command entries, as word prefixes of a bash segment's effective
/// words: `(label, words, reason)`. `git commit` and `git add` are for a
/// step that only reads and reports; a writing step commits.
const COMMANDS: &[(&str, &[&str], &str)] = &[
    ("git push", &["git", "push"], PUSH_REASON),
    ("git rebase", &["git", "rebase"], TREE_REASON),
    ("git reset", &["git", "reset"], TREE_REASON),
    ("git checkout --", &["git", "checkout", "--"], TREE_REASON),
    ("git stash", &["git", "stash"], TREE_REASON),
    ("git clean", &["git", "clean"], TREE_REASON),
    ("git add -A", &["git", "add", "-A"], TREE_REASON),
    ("git commit", &["git", "commit"], READ_ONLY_REASON),
    ("git add", &["git", "add"], READ_ONLY_REASON),
];

/// The one symbolic entry: the `.env` file, read or written.
const COPY_ENV: &str = "copy .env";

/// The rule a command word that cannot be read denies with.
const UNREADABLE: &str = "unreadable";

const PUSH_REASON: &str =
    "the runner pushes once after its checks (PLAN-layer2 §6.4); commit and call finish_step";

const TREE_REASON: &str =
    "the tree is the runner's evidence; commit your work and call finish_step";

const READ_ONLY_REASON: &str =
    "this step reads and reports; it changes nothing in git, so call finish_step with your report";

const ENV_REASON: &str =
    "a step thread never reads or writes .env (#41); the key must not reach the log (AGENTS.md)";

const UNREADABLE_REASON: &str = "keep commands literal in a step thread";

/// A deny list the overlay's grammar does not recognise. Named loudly
/// where the overlay is built, so a workflow typo never becomes a call
/// the runner thought was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyParseError {
    /// The entry as it was written.
    pub entry: String,
}

impl fmt::Display for DenyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown step deny entry `{}`: one of {} or `{}`",
            self.entry,
            COMMANDS
                .iter()
                .map(|(label, _, _)| format!("`{label}`"))
                .collect::<Vec<_>>()
                .join(", "),
            COPY_ENV
        )
    }
}

impl std::error::Error for DenyParseError {}

/// The parsed deny list of one step thread.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StepOverlay {
    /// The command entries the list names, in the list's order.
    commands: Vec<(&'static str, &'static [&'static str], &'static str)>,
    /// Whether the list carries the `.env` entry.
    copy_env: bool,
}

impl StepOverlay {
    /// Parse a workflow's `deny` list.
    pub fn parse(deny: &[String]) -> Result<Self, DenyParseError> {
        let mut overlay = StepOverlay::default();
        for entry in deny {
            // Written either way — `git  push`, ` git add -A ` — the
            // entry means the same command.
            let text = entry.split_whitespace().collect::<Vec<_>>().join(" ");
            if let Some(&(label, words, reason)) =
                COMMANDS.iter().find(|(label, _, _)| *label == text)
            {
                overlay.commands.push((label, words, reason));
            } else if text == COPY_ENV {
                overlay.copy_env = true;
            } else {
                return Err(DenyParseError {
                    entry: entry.clone(),
                });
            }
        }
        Ok(overlay)
    }

    /// What the overlay says about a call, whatever the rules and the
    /// mode say: `None` means the call is not its business, a `Deny` is
    /// final.
    pub fn decide(&self, call: &ToolCall) -> Option<Outcome> {
        if call.name == "bash" {
            let command = call.args.get("command")?.as_str()?;
            let reads = segment_reads(command);
            // A command word that cannot be read could be any entry, so
            // it is refused first: no entry may be talked past by making
            // the line unreadable, and `cat $( git push )` stays an entry
            // deny because *its* segments are readable.
            if reads.iter().any(|read| read.unreadable) {
                return Some(deny(UNREADABLE, UNREADABLE_REASON));
            }
            if let Some((label, reason)) = self.command(&reads) {
                return Some(deny(label, reason));
            }
            if self.copy_env && env_in_bash(&reads) {
                return Some(deny(COPY_ENV, ENV_REASON));
            }
            None
        } else if self.copy_env && env_path(call) {
            Some(deny(COPY_ENV, ENV_REASON))
        } else {
            None
        }
    }

    /// The entry with the most words a segment matches; a tie goes to
    /// the earlier entry. A segment matches when its effective words
    /// start with the entry's words, so `git push` covers
    /// `git push --force` and never `git pushy`.
    fn command(&self, reads: &[SegRead]) -> Option<(&'static str, &'static str)> {
        let mut best: Option<(&'static str, usize, &'static str)> = None;
        for (label, words, reason) in &self.commands {
            let matched = reads.iter().any(|read| starts_with(&read.words, words));
            let better = match best {
                None => true,
                Some((_, len, _)) => words.len() > len,
            };
            if matched && better {
                best = Some((label, words.len(), *reason));
            }
        }
        best.map(|(label, _, reason)| (label, reason))
    }
}

fn starts_with(words: &[String], entry: &[&str]) -> bool {
    words.len() >= entry.len()
        && entry
            .iter()
            .zip(words)
            .all(|(entry, word)| *entry == word.as_str())
}

fn deny(entry: &str, reason: &str) -> Outcome {
    Outcome::Deny {
        rule: format!("step deny {entry}"),
        reason: reason.to_owned(),
    }
}

/// The `.env` file, by its final path component and nothing else:
/// `.env.example` and `dir.env/x` never match.
fn is_env(word: &str) -> bool {
    word == ".env" || word.ends_with("/.env")
}

/// A bash segment carrying the literal word or redirection target `.env`
/// — the rule is literal-only, so `$DIR/.env` is not a path it can read —
/// unless the segment's command word is `.` or `source`, the two ways the
/// pty check loads the key it was given by absolute path.
fn env_in_bash(reads: &[SegRead]) -> bool {
    reads.iter().any(|read| {
        let sources = read
            .words
            .first()
            .is_some_and(|w| w == "." || w == "source");
        !sources && read.literal.iter().any(|w| is_env(w))
    })
}

/// The three tools whose `path` argument reads or writes a file.
fn env_path(call: &ToolCall) -> bool {
    if !matches!(call.name.as_str(), "read_file" | "write_file" | "edit_file") {
        return false;
    }
    let Some(path) = call.args.get("path").and_then(|p| p.as_str()) else {
        return false;
    };
    is_env(path.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Policy;
    use crate::shell::{FORWARDERS, Forwarder, GIT_FLAGS, GIT_VALUES};
    use aigentic_core::{RiskClass, ToolCall};
    use serde_json::json;

    /// The overlay over an empty rule set: every outcome here comes from
    /// the overlay, so the sibling policy is the A/B and the rule name is
    /// the assertion.
    fn policy_with(deny_list: &[&str]) -> StepOverlay {
        StepOverlay::parse(&strings(deny_list)).unwrap()
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn bash(command: &str) -> ToolCall {
        ToolCall {
            id: "c".into(),
            name: "bash".into(),
            args: json!({"command": command}),
        }
    }

    fn call(name: &str, path: &str) -> ToolCall {
        ToolCall {
            id: "c".into(),
            name: name.into(),
            args: json!({"path": path}),
        }
    }

    /// The rule an outcome names, or its kind when it has no rule.
    fn rule(outcome: &Outcome) -> String {
        match outcome {
            Outcome::Allow { rule } => format!("allow {rule}"),
            Outcome::Ask { .. } | Outcome::AskBoundary { .. } => "ask".into(),
            Outcome::Deny { rule, .. } => rule.clone(),
        }
    }

    fn denied(overlay: &StepOverlay, call: &ToolCall) -> String {
        match overlay.decide(call) {
            Some(outcome @ Outcome::Deny { .. }) => rule(&outcome),
            other => panic!("expected a deny, got {other:?}"),
        }
    }

    /// Each entry as a lone command line. The expectation is built from
    /// the same list the overlay parses, so no entry's name is written
    /// twice.
    #[test]
    fn t1_each_entry_denies_a_lone_command_line() {
        for (label, words, _) in COMMANDS {
            let overlay = policy_with(&[label]);
            let command = words.join(" ");
            assert_eq!(
                denied(&overlay, &bash(&command)),
                format!("step deny {label}")
            );
        }
    }

    /// Each entry as a later segment of a compound line.
    #[test]
    fn t2_each_entry_denies_as_a_later_segment() {
        for (label, words, _) in COMMANDS {
            let overlay = policy_with(&[label]);
            let command = format!("cargo test && {}", words.join(" "));
            assert_eq!(
                denied(&overlay, &bash(&command)),
                format!("step deny {label}")
            );
        }
    }

    /// The ways a line can hide an entry: shell plumbing that still runs
    /// the same command, each naming the entry it hides.
    #[test]
    fn t3_the_bypass_pack_names_the_inner_entry() {
        let overlay = policy_with(&["git push", "git rebase", "git add -A"]);
        let cases: &[(&str, &str)] = &[
            ("git -C /tmp push", "git push"),
            ("git --git-dir=/t/g push", "git push"),
            ("env git push", "git push"),
            ("command git push", "git push"),
            ("sh -c 'git push'", "git push"),
            (
                "bash -c \"cargo test && git rebase -i HEAD~2\"",
                "git rebase",
            ),
            ("git add --all", "git add -A"),
            ("echo $(git push)", "git push"),
        ];
        for (command, entry) in cases {
            assert_eq!(
                denied(&overlay, &bash(command)),
                format!("step deny {entry}"),
                "{command}"
            );
        }
    }

    /// A command word nobody can read is refused as unreadable, while a
    /// readable segment whose substitution holds the entry names the
    /// entry (the plan amendment 1 correction: `cat` is literal, so the
    /// segment is read and its substitution is recursed into).
    #[test]
    fn t4_unreadable_command_words_deny_as_unreadable() {
        let overlay = policy_with(&["git push"]);
        for command in ["cmd=$X; $cmd push", "bash -c $SCRIPT", "$cmd push"] {
            assert_eq!(
                denied(&overlay, &bash(command)),
                "step deny unreadable",
                "{command}"
            );
        }
        assert_eq!(
            denied(&overlay, &bash("cat $( git push )")),
            "step deny git push"
        );
    }

    /// The A/B: a command the overlay does not cover reads exactly as it
    /// does with no overlay at all, so a step thread is an ordinary
    /// thread for everything else. `git commit --amend` is the caller's
    /// own example of work that stays allowed. The last two rows reach the
    /// command word past a wrapper's stop option and past a global option,
    /// neither of which is the command.
    #[test]
    fn t5_an_uncovered_command_reads_as_without_the_overlay() {
        let overlay = StepOverlay::parse(&strings(&["git push"])).unwrap();
        let plain = Policy::defaults();
        for command in [
            "git commit --amend -m x",
            "cargo test",
            "git log --oneline -3",
            "command -v git push",
            "git --no-pager log",
        ] {
            let call = bash(command);
            let plain = plain.decide(&call, RiskClass::Exec, None);
            assert!(!matches!(plain, Outcome::Deny { .. }), "{command}");
            assert_eq!(overlay.decide(&call), None, "{command}");
        }
    }

    /// The `.env` entry: the file by its final component, its two
    /// sourcing forms exempt, and the instructive contrast.
    #[test]
    fn t6_the_dot_env_rule() {
        let overlay = policy_with(&[COPY_ENV]);
        let plain = Policy::defaults();
        for command in [
            "cp .env /tmp/41-pty",
            "cat .env",
            "echo x >> .env",
            "cat /repo/.env",
        ] {
            assert_eq!(
                denied(&overlay, &bash(command)),
                "step deny copy .env",
                "{command}"
            );
        }
        for (name, path) in [
            ("read_file", ".env"),
            ("write_file", "sub/.env"),
            ("edit_file", "/repo/.env"),
        ] {
            let call = call(name, path);
            assert_eq!(
                denied(&overlay, &call),
                "step deny copy .env",
                "{name} {path}"
            );
        }
        // The exemption and the exact-component rule are the A/B: the
        // overlay says nothing about these, so `None` comes out and the
        // caller falls through to the sibling policy, which does not
        // deny any of them.
        for command in [
            ". /repo/.env",
            "source /repo/.env",
            "cat .env.example",
            "echo $DIR/.env",
        ] {
            let call = bash(command);
            assert!(
                !matches!(
                    plain.decide(&call, RiskClass::Exec, None),
                    Outcome::Deny { .. }
                ),
                "{command} is not denied without the overlay"
            );
            assert_eq!(overlay.decide(&call), None, "{command}");
        }
        let call = call("read_file", ".env.example");
        assert!(
            !matches!(
                plain.decide(&call, RiskClass::Read, None),
                Outcome::Deny { .. }
            ),
            "the example file is not denied without the overlay"
        );
        assert_eq!(overlay.decide(&call), None);
    }

    /// A step that only reports may not commit or stage anything, and its
    /// refusal says so rather than asking it to commit. The longer entry
    /// still wins where both are listed.
    #[test]
    fn a_reporting_step_may_not_commit_or_stage() {
        let overlay = policy_with(&["git commit", "git add", "git add -A"]);
        for (command, rule) in [
            ("git commit -m x", "step deny git commit"),
            ("git add src/lib.rs", "step deny git add"),
            ("git add -A", "step deny git add -A"),
        ] {
            match overlay.decide(&bash(command)) {
                Some(Outcome::Deny { rule: got, reason }) => {
                    assert_eq!(got, rule, "{command}");
                    assert_eq!(
                        reason.contains("reads and reports"),
                        rule != "step deny git add -A",
                        "{command}: {reason}"
                    );
                }
                other => panic!("{command}: {other:?}"),
            }
        }
        assert_eq!(overlay.decide(&bash("git commit-tree x")), None);
        assert_eq!(overlay.decide(&bash("git log")), None);
    }

    /// Anything outside the grammar is named where the overlay is built.
    #[test]
    fn t7_parse_rejects_an_unknown_entry() {
        let err = StepOverlay::parse(&strings(&["git obliterate"])).unwrap_err();
        assert_eq!(err.entry, "git obliterate");
        assert!(err.to_string().contains("git obliterate"), "{err}");
    }

    /// The positional words a row's wrapper takes of its own, as the `1`
    /// placeholders a line writes them with. `timeout`'s duration is the
    /// only one today.
    fn positional_filler(row: &Forwarder) -> String {
        " 1".repeat(row.positional)
    }

    /// Every option [`FORWARDERS`] lists is read past its wrapper, in
    /// every spelling the reader handles, so the command word behind it
    /// is the one the overlay matches.
    #[test]
    fn t8_every_listed_forwarder_option_reads_past_the_forwarder() {
        let overlay = policy_with(&["git push"]);
        for row in FORWARDERS {
            let filler = positional_filler(row);
            for opt in row.values {
                let mut lines = vec![format!("{} {opt} v{filler} git push", row.name)];
                if opt.starts_with("--") {
                    lines.push(format!("{} {opt}=v{filler} git push", row.name));
                } else {
                    lines.push(format!("{} {opt}v{filler} git push", row.name));
                }
                for line in lines {
                    assert_eq!(
                        denied(&overlay, &bash(&line)),
                        "step deny git push",
                        "{line}"
                    );
                }
            }
            for opt in row.flags {
                let line = format!("{} {opt}{filler} git push", row.name);
                assert_eq!(
                    denied(&overlay, &bash(&line)),
                    "step deny git push",
                    "{line}"
                );
            }
        }
    }

    /// Every entry denies behind every wrapper, so the unwrapping is a
    /// property of the reader and not of the push.
    #[test]
    fn t9_a_wrapped_entry_denies_under_every_forwarder() {
        for row in FORWARDERS {
            let filler = positional_filler(row);
            for (label, words, _) in COMMANDS {
                let overlay = policy_with(&[label]);
                let line = format!("{}{filler} {}", row.name, words.join(" "));
                assert_eq!(
                    denied(&overlay, &bash(&line)),
                    format!("step deny {label}"),
                    "{line}"
                );
            }
        }
    }

    /// Which words name a program: a path's final component decides, and
    /// quoting it changes nothing, because the shell strips the quotes
    /// before the name is used. The words that merely contain a name do
    /// not match it.
    #[test]
    fn t10_a_command_word_matches_by_its_final_path_component() {
        let overlay = policy_with(&["git push"]);
        for line in [
            "/usr/bin/git push",
            "./git push",
            "dir/git push",
            "\"/usr/bin/git\" push",
            "\\git push",
            "\"git\" push",
            "\"sudo\" git push",
            "/usr/bin/sudo /usr/bin/git push",
            "/usr/bin/env git push",
            "/bin/sh -c 'git push'",
        ] {
            assert_eq!(
                denied(&overlay, &bash(line)),
                "step deny git push",
                "{line}"
            );
        }
        for line in [
            "mygit push",
            "foo.git push",
            "git.exe push",
            "git pushy",
            "echo /usr/bin/git push",
        ] {
            assert_eq!(overlay.decide(&bash(line)), None, "{line}");
        }
    }

    /// An option's value is not the command word: a reader that took the
    /// first option word for the command would read `git` here and deny a
    /// line that runs `push`.
    #[test]
    fn t11_a_forwarders_value_word_is_not_its_command_word() {
        let overlay = policy_with(&["git push"]);
        for row in FORWARDERS.iter().filter(|r| !r.values.is_empty()) {
            let filler = positional_filler(row);
            for opt in row.values {
                let line = format!("{} {opt} git{filler} push", row.name);
                assert_eq!(overlay.decide(&bash(&line)), None, "{line}");
            }
        }
    }

    /// An option no table knows may be taking the next word as its value,
    /// which is exactly the hiding the reader closes: the line is refused
    /// rather than read past. So is a wrapper whose own word is missing.
    #[test]
    fn t12_an_option_no_table_knows_refuses_the_line() {
        let overlay = policy_with(&["git push"]);
        for line in [
            "sudo --definitely-not-a-sudo-option git push",
            "timeout --whatever 5 git push",
            "git --whatever push",
            "env -Q git push",
            "sudo -u",
            "timeout",
        ] {
            assert_eq!(
                denied(&overlay, &bash(line)),
                "step deny unreadable",
                "{line}"
            );
        }
    }

    /// `git`'s own global options stand between the name and the command
    /// word; the reader steps over each spelling of each of them.
    #[test]
    fn t13_every_listed_git_global_option_reads_past_git() {
        let overlay = policy_with(&["git push"]);
        let mut lines = Vec::new();
        for opt in GIT_VALUES {
            lines.push(format!("git {opt} v push"));
            if opt.starts_with("--") {
                lines.push(format!("git {opt}=v push"));
            } else {
                lines.push(format!("git {opt}v push"));
            }
        }
        for flag in GIT_FLAGS {
            lines.push(format!("git {flag} push"));
        }
        for line in lines {
            assert_eq!(
                denied(&overlay, &bash(&line)),
                "step deny git push",
                "{line}"
            );
        }
    }

    /// A wrapper's stop option means it runs no command of its own, so
    /// nothing after it is a command word. The two `sudo` options that
    /// look like stops but are not (`-k`, `--reset-timestamp`) run the
    /// command, and are listed as flags.
    #[test]
    fn t14_an_option_that_runs_nothing_ends_the_read() {
        let overlay = policy_with(&["git push"]);
        for row in FORWARDERS {
            for opt in row.stops {
                let line = format!("{} {opt} git push", row.name);
                assert_eq!(overlay.decide(&bash(&line)), None, "{line}");
            }
        }
        for line in [
            "sudo -l git push",
            "sudo -K git push",
            "command -v git push",
            "command -V git push",
            "timeout --help git push",
        ] {
            assert_eq!(overlay.decide(&bash(line)), None, "{line}");
        }
        for line in ["sudo -k git push", "sudo --reset-timestamp git push"] {
            assert_eq!(
                denied(&overlay, &bash(line)),
                "step deny git push",
                "{line}"
            );
        }
    }

    /// The entry table is what the tests above say it is. `t1`, `t2` and
    /// `t9` derive their rows from `COMMANDS`, so without this pin
    /// dropping an entry would shrink the parser and its tests together.
    #[test]
    fn t15_the_entry_table_matches_the_list_written_out() {
        let expected: &[(&str, &[&str], &str)] = &[
            ("git push", &["git", "push"], PUSH_REASON),
            ("git rebase", &["git", "rebase"], TREE_REASON),
            ("git reset", &["git", "reset"], TREE_REASON),
            ("git checkout --", &["git", "checkout", "--"], TREE_REASON),
            ("git stash", &["git", "stash"], TREE_REASON),
            ("git clean", &["git", "clean"], TREE_REASON),
            ("git add -A", &["git", "add", "-A"], TREE_REASON),
            ("git commit", &["git", "commit"], READ_ONLY_REASON),
            ("git add", &["git", "add"], READ_ONLY_REASON),
        ];
        assert_eq!(COMMANDS, expected);
    }

    /// One row per form the module doc's limits paragraph names, quoted
    /// here as the rows' source:
    ///
    /// > Stated limits: a command reached through `xargs`, `find -exec`,
    /// > `eval` or `python -c`; a wrapper with no row in `FORWARDERS`
    /// > (`doas`, `setsid`, `run0`, `ionice`, `chrt`, `flock`,
    /// > `unbuffer`, `script`) or a multi-call binary that chooses its
    /// > command from its own words (`busybox git push`); `env -S <line>`,
    /// > whose value is a command line the reader does not split; a
    /// > here-document body fed to a shell (`bash <<EOF` with the command
    /// > inside); `PATH` shadowing, where the reader reads the word `git`
    /// > and never the program it resolves to; and `git pull --rebase`,
    /// > which names no entry yet.
    ///
    /// A form that stays outside the guard reads as `None`; the `PATH`
    /// row denies because the reader reads the word and never the program
    /// it resolves to. A change that closes one of these forms fails a row,
    /// which is where the paragraph is updated with it. The paragraph's
    /// clause about a word that mixes literal text with a substitution is
    /// pinned by `t4`.
    #[test]
    fn t16_the_stated_limits_still_escape_the_overlay() {
        let overlay = policy_with(&["git push"]);
        for line in [
            "xargs git push",
            "find . -exec git push \\;",
            "eval 'git push'",
            "python3 -c 'print(1)'",
            "busybox git push",
            "doas git push",
            "setsid git push",
            "run0 git push",
            "ionice git push",
            "chrt 5 git push",
            "flock /tmp/lock git push",
            "unbuffer git push",
            "script -q /dev/null git push",
            "env -S 'git push'",
            "bash <<'EOF'\ngit push\nEOF",
            "git pull --rebase",
        ] {
            assert_eq!(overlay.decide(&bash(line)), None, "{line}");
        }
        assert_eq!(
            denied(&overlay, &bash("PATH=/tmp/bin:$PATH git push")),
            "step deny git push"
        );
    }
}
