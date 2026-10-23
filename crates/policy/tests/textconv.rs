//! A repository's own git config is trusted on the default path.
//!
//! Plain `git diff`, `git log -p` and `git show` run without asking even
//! when the repository names a textconv or an external diff driver,
//! because the config that names that program can only arrive through an
//! edit or write tool, which already asks. The flag that names the filter
//! itself, `--textconv`, still asks.
//!
//! The fixture is a real repository whose driver prints [`MARKER`], so a
//! command that never reached the driver cannot pass for one that did,
//! and every child runs with the caller's own git config out of the way.

use std::path::{Path, PathBuf};
use std::process::Command;

use aigentic_core::{RiskClass, ToolCall};
use aigentic_policy::{Decision, Outcome, Policy, default_bash_allow, default_rules};
use serde_json::json;

/// What the fixture's driver prints; every "the driver ran" assertion is
/// this word appearing in the command's stdout.
const MARKER: &str = "textconv-driver-ran";

/// The commands whose plain form must stay read-only, with the arguments
/// git is given for each.
const PLAIN: [(&str, &[&str]); 3] = [
    ("git diff", &["diff"]),
    ("git log -p -1", &["log", "-p", "-1"]),
    ("git show", &["show"]),
];

/// The summary forms, pinned for their verdict only: they reach no
/// driver's output by accident.
const SUMMARY: [&str; 3] = ["git diff --stat", "git log --oneline", "git show --stat"];

fn bash(command: &str) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        name: "bash".into(),
        args: json!({ "command": command }),
    }
}

/// What the default table says about `command` run from `repo`.
fn decided(command: &str, repo: &Path) -> Outcome {
    Policy::defaults().decide(&bash(command), RiskClass::Exec, Some(repo))
}

/// The rule name a default-allow bash call records: the one default
/// pattern the command starts with.
fn allow_rule(command: &str) -> String {
    let mut matches = default_bash_allow()
        .into_iter()
        .filter(|pattern| command == pattern || command.starts_with(&format!("{pattern} ")));
    let pattern = matches
        .next()
        .unwrap_or_else(|| panic!("no default allow pattern starts {command}"));
    assert!(
        matches.next().is_none(),
        "more than one default pattern starts {command}"
    );
    format!("bash allow-pattern {pattern}")
}

/// The reason an exec-class refusal to allow carries: the exec ask row's
/// own name and reason.
fn ask_reason() -> String {
    let mut rows = default_rules()
        .into_iter()
        .filter(|rule| rule.class == Some(RiskClass::Exec) && rule.decision == Decision::Ask);
    let row = rows.next().expect("the default table asks on exec");
    assert!(rows.next().is_none(), "one exec ask row");
    format!("{}: {}", row.name(), row.reason)
}

/// A scratch repository plus the program its config can name as a diff
/// driver.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    /// A repository whose `.gitattributes` commit carries
    /// `*.bin diff=marker`, the line that makes git look for a
    /// `diff.marker.*` driver.
    fn with_attributes() -> Self {
        Self::build(true)
    }

    /// The same repository with no `.gitattributes`, so only a
    /// `diff.external` can reach a program.
    fn without_attributes() -> Self {
        Self::build(false)
    }

    fn build(attributes: bool) -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        let fixture = Self { dir };
        let repo = fixture.repo();
        std::fs::create_dir(&repo).expect("the repository directory");
        std::fs::write(
            fixture.marker(),
            format!("#!/bin/sh\necho \"{MARKER}: $1\"\n"),
        )
        .expect("the marker program");
        set_executable(&fixture.marker());

        fixture.git(&["init", "-q"]);
        fixture.git(&["config", "user.email", "t@example.com"]);
        fixture.git(&["config", "user.name", "Test"]);
        // A NUL byte makes git read the file as binary, so a plain form
        // that never reached the driver shows a notice and no marker.
        std::fs::write(repo.join("a.bin"), b"\0\0\0a\n").expect("the committed content");
        let mut added = vec!["add", "a.bin"];
        if attributes {
            std::fs::write(repo.join(".gitattributes"), "*.bin diff=marker\n")
                .expect("the attributes file");
            added.push(".gitattributes");
        }
        fixture.git(&added);
        fixture.git(&["commit", "-q", "-m", "one"]);
        std::fs::write(repo.join("a.bin"), b"\0\0\0b\n").expect("the worktree change");
        fixture
    }

    fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn marker(&self) -> PathBuf {
        self.dir.path().join("marker")
    }

    /// Name the marker program in the repository's own config, under the
    /// key a driver is read from: `diff.marker.textconv` or
    /// `diff.external`.
    fn configure(&self, key: &str) {
        let program = self.marker();
        self.git(&["config", key, program.to_str().expect("a utf-8 path")]);
    }

    /// Run git in the fixture and return its stdout.
    ///
    /// `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1` keep a
    /// developer's or a runner's own config (a stray `diff.external`, a
    /// `textconv.cache`) from deciding the result, and `GIT_DIR` and its
    /// neighbours are cleared so no enclosing repository claims the run.
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(self.repo())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .expect("the marker's mode")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("the marker is executable");
}

#[test]
fn plain_and_summary_git_forms_stay_read_only_with_a_textconv_driver_configured() {
    let fixture = Fixture::with_attributes();
    let repo = fixture.repo();
    let plain: Vec<Outcome> = PLAIN
        .iter()
        .map(|(command, _)| decided(command, &repo))
        .collect();
    for ((command, _), outcome) in PLAIN.iter().zip(&plain) {
        assert_eq!(
            outcome,
            &Outcome::Allow {
                rule: allow_rule(command)
            },
            "{command} runs without asking with no driver configured"
        );
    }

    fixture.configure("diff.marker.textconv");
    for (command, args) in PLAIN {
        let stdout = fixture.git(args);
        assert!(
            stdout.contains(MARKER),
            "{command} reaches the configured driver: {stdout}"
        );
    }
    let after: Vec<Outcome> = PLAIN
        .iter()
        .map(|(command, _)| decided(command, &repo))
        .collect();
    assert_eq!(
        after, plain,
        "configuring a driver changes what git runs, not what policy says"
    );

    for command in SUMMARY {
        assert_eq!(
            decided(command, &repo),
            Outcome::Allow {
                rule: allow_rule(command)
            },
            "{command} runs without asking in the configured repository"
        );
    }
}

#[test]
fn the_textconv_flags_ask_while_the_plain_forms_stay_read_only() {
    let fixture = Fixture::with_attributes();
    let repo = fixture.repo();
    fixture.configure("diff.marker.textconv");

    for command in [
        "git diff --textconv",
        "git log -p --textconv",
        "git show --textconv",
    ] {
        assert_eq!(
            decided(command, &repo),
            Outcome::Ask {
                reason: ask_reason()
            },
            "{command} names the filter itself"
        );
    }
    for command in ["git diff --no-textconv", "git diff --stat"] {
        assert_eq!(
            decided(command, &repo),
            Outcome::Allow {
                rule: allow_rule(command)
            },
            "{command} names no filter"
        );
    }
}

#[test]
fn a_configured_external_diff_keeps_plain_git_diff_read_only() {
    let fixture = Fixture::without_attributes();
    let repo = fixture.repo();
    fixture.configure("diff.external");

    let stdout = fixture.git(&["diff"]);
    assert!(
        stdout.contains(MARKER),
        "plain git diff reaches the external driver: {stdout}"
    );
    assert_eq!(
        decided("git diff", &repo),
        Outcome::Allow {
            rule: allow_rule("git diff")
        },
        "an external diff configured in the repository does not make git diff ask"
    );
}
