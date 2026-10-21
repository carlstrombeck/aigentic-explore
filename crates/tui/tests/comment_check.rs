//! `scripts/comment-check.sh` keeps added Rust comments inside `AGENTS.md`'s
//! "Comments" rule. These tests pin its contract: which added lines fail, the
//! rule names it prints, the new-side line number it reports, and that a base
//! it cannot resolve is an error rather than a pass, since a check that cannot
//! run must never look like one that found nothing.

use std::fs;
use std::process::{Command, Output};

use tempfile::TempDir;

const CHECK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/comment-check.sh"
);
const USAGE: &str = "usage: scripts/comment-check.sh <base>";

/// The rule names are the script's output contract, so every expectation is
/// spelled from these.
const ISSUE_NUMBER: &str = "issue or PR number";
const ISSUE_REFERENCE: &str = "issue reference";
const TEST_PLAN_LABEL: &str = "test-plan label";
const AS_BEFORE: &str = "history phrase \"as before\"";
const USED_TO: &str = "history phrase \"used to\"";
const NOW_THAT: &str = "history phrase \"now that\"";
const HEAD_STATE: &str = "history phrase \"HEAD's\"";

/// A fixture repository in its own temp dir. Each git call and each run of the
/// check carries the isolation `crates/policy/tests/textconv.rs` uses, so a
/// developer's own configuration cannot decide a fixture's result, and nothing
/// here reaches the repository the test itself runs in.
struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        let repo = Self { dir };
        repo.git_ok(&["init", "-q"]);
        repo.git_ok(&["config", "user.email", "t@example.com"]);
        repo.git_ok(&["config", "user.name", "Test"]);
        repo
    }

    /// Write `lines` to `path`, one line each, with a newline after the last.
    fn write(&self, path: &str, lines: &[&str]) {
        let full = self.dir.path().join(path);
        fs::create_dir_all(full.parent().expect("a parent directory"))
            .expect("the fixture directory");
        fs::write(&full, format!("{}\n", lines.join("\n"))).expect("the fixture file");
    }

    /// Commit everything the fixture holds. Staging all of it is safe here: the
    /// tree is a throwaway this test created and wrote.
    fn commit(&self, message: &str) {
        self.git_ok(&["add", "--all"]);
        self.git_ok(&["commit", "-q", "-m", message]);
    }

    /// Run the check with the fixture's root as its working directory.
    fn run(&self, args: &[&str]) -> Output {
        self.run_in(".", args)
    }

    /// Run the check from `cwd`, relative to the fixture's root.
    fn run_in(&self, cwd: &str, args: &[&str]) -> Output {
        Command::new(CHECK)
            .current_dir(self.dir.path().join(cwd))
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("the check runs")
    }

    fn git(&self, args: &[&str]) -> Output {
        Command::new("git")
            .current_dir(self.dir.path())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("git runs")
    }

    fn git_ok(&self, args: &[&str]) {
        let out = self.git(args);
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn status(output: &Output) -> i32 {
    output.status.code().expect("the check exits with a code")
}

/// The 1-based index of the first line of `lines` holding `needle`. Every
/// expected line number is this, never a number typed by hand.
fn line_of(lines: &[&str], needle: &str) -> usize {
    lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no fixture line holds {needle:?}"))
        + 1
}

/// Assert the check reported exactly this one hit line and failed.
fn assert_one_hit(out: &Output, expected: &str) {
    assert_eq!(status(out), 1, "stderr: {}", stderr(out));
    assert_eq!(stdout(out), format!("{expected}\n"));
    assert_eq!(
        stderr(out),
        "comment-check: 1 hits in 1 added comment lines\n"
    );
}

/// Assert the check found nothing and passed.
fn assert_clean(out: &Output) {
    assert_eq!(status(out), 0, "stderr: {}", stderr(out));
    assert_eq!(stdout(out), "");
    assert_eq!(
        stderr(out),
        "comment-check: 0 hits in 0 added comment lines\n"
    );
}

/// A fixture whose `src/lib.rs` starts with one code line, gains `added` over
/// it, and is then checked against that base. Returns the output and the
/// head lines, which the caller passes to `line_of`.
fn added_to_lib(added: &[&'static str]) -> (Output, Vec<&'static str>) {
    let repo = Repo::new();
    repo.write("src/lib.rs", &["fn a() {}"]);
    repo.commit("base");
    let mut head: Vec<&'static str> = vec!["fn a() {}"];
    head.extend_from_slice(added);
    repo.write("src/lib.rs", &head);
    repo.commit("head");
    (repo.run(&["HEAD~1"]), head)
}

#[test]
fn issue_numbers_and_references_fail_with_their_line() {
    let (out, head) = added_to_lib(&["// issue #12: keep the ordering"]);
    assert_one_hit(
        &out,
        &format!(
            "src/lib.rs:{}: {ISSUE_NUMBER}: // issue #12: keep the ordering",
            line_of(&head, "#12")
        ),
    );

    let (out, head) = added_to_lib(&["// see #7 for the reasoning"]);
    assert_one_hit(
        &out,
        &format!(
            "src/lib.rs:{}: {ISSUE_NUMBER}: // see #7 for the reasoning",
            line_of(&head, "#7")
        ),
    );

    let (out, head) = added_to_lib(&["// see the issue # for the reasoning"]);
    assert_one_hit(
        &out,
        &format!(
            "src/lib.rs:{}: {ISSUE_REFERENCE}: // see the issue # for the reasoning",
            line_of(&head, "issue #")
        ),
    );
}

#[test]
fn a_test_plan_label_fails_in_every_shape() {
    let (out, head) = added_to_lib(&["/// T3 (issue #115): the developer view"]);
    assert_one_hit(
        &out,
        &format!(
            "src/lib.rs:{}: {ISSUE_NUMBER}, {TEST_PLAN_LABEL}: /// T3 (issue #115): the developer view",
            line_of(&head, "#115")
        ),
    );

    let (out, head) = added_to_lib(&["// T6: the last line"]);
    assert_one_hit(
        &out,
        &format!(
            "src/lib.rs:{}: {TEST_PLAN_LABEL}: // T6: the last line",
            line_of(&head, "T6:")
        ),
    );

    let (out, head) = added_to_lib(&["// pinned by T4 (two fixtures): the shape stays one rule"]);
    assert_one_hit(
        &out,
        &format!(
            "src/lib.rs:{}: {TEST_PLAN_LABEL}: // pinned by T4 (two fixtures): the shape stays one rule",
            line_of(&head, "T4 (")
        ),
    );
}

#[test]
fn a_history_phrase_fails_and_is_named() {
    let (out, head) = added_to_lib(&["/// The width is only used to keep bullet lines sane."]);
    assert_one_hit(
        &out,
        &format!(
            "src/lib.rs:{}: {USED_TO}: /// The width is only used to keep bullet lines sane.",
            line_of(&head, "The width")
        ),
    );

    let (out, head) = added_to_lib(&["// As before, the loop retries once."]);
    assert_one_hit(
        &out,
        &format!(
            "src/lib.rs:{}: {AS_BEFORE}: // As before, the loop retries once.",
            line_of(&head, "As before")
        ),
    );

    let (out, head) = added_to_lib(&["// Now that HEAD's state is loaded, the loop resumes."]);
    assert_one_hit(
        &out,
        &format!(
            "src/lib.rs:{}: {NOW_THAT}, {HEAD_STATE}: // Now that HEAD's state is loaded, the loop resumes.",
            line_of(&head, "Now that")
        ),
    );
}

#[test]
fn a_comment_that_follows_the_rule_passes() {
    let (out, _head) = added_to_lib(&[
        "/// What the gate refuses: a request with no project.",
        "fn b() {}",
    ]);
    assert_eq!(status(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(
        stderr(&out),
        "comment-check: 0 hits in 1 added comment lines\n"
    );
}

#[test]
fn an_existing_comment_at_the_base_never_fails() {
    let repo = Repo::new();
    repo.write(
        "src/lib.rs",
        &[
            "// issue #9: the older note",
            "// T2: the older note",
            "fn a() {}",
        ],
    );
    repo.commit("base");
    repo.write(
        "src/lib.rs",
        &[
            "// issue #9: the older note",
            "// T2: the older note",
            "fn a() -> u8 { 1 }",
        ],
    );
    repo.commit("head");

    assert_clean(&repo.run(&["HEAD~1"]));
}

#[test]
fn the_escape_marker_clears_the_line() {
    let (out, _head) = added_to_lib(&["// see https://example.com/x#12 // comment-check: allow"]);
    assert_eq!(status(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(
        stderr(&out),
        "comment-check: 0 hits in 0 added comment lines\n"
    );

    let (out, head) = added_to_lib(&["// see https://example.com/x#12"]);
    assert_one_hit(
        &out,
        &format!(
            "src/lib.rs:{}: {ISSUE_NUMBER}: // see https://example.com/x#12",
            line_of(&head, "example.com")
        ),
    );
}

#[test]
fn the_reported_line_is_the_new_side_line() {
    let base: Vec<String> = (1..=20).map(|n| format!("let n{n} = 0;")).collect();
    let base_lines: Vec<&str> = base.iter().map(String::as_str).collect();
    let replaced = line_of(&base_lines, "n10 ");

    let repo = Repo::new();
    repo.write("src/lib.rs", &base_lines);
    repo.commit("base");

    let mut head: Vec<String> = Vec::new();
    for (position, line) in base.iter().enumerate() {
        if position + 1 == replaced {
            head.push("let ten = 10;".to_string());
            head.push("// issue #12: the tenth line".to_string());
        } else {
            head.push(line.clone());
        }
    }
    head.push("let extra = 0;".to_string());
    let head_lines: Vec<&str> = head.iter().map(String::as_str).collect();
    repo.write("src/lib.rs", &head_lines);
    repo.commit("head");

    assert_one_hit(
        &repo.run(&["HEAD~1"]),
        &format!(
            "src/lib.rs:{}: {ISSUE_NUMBER}: // issue #12: the tenth line",
            line_of(&head_lines, "#12")
        ),
    );
}

#[test]
fn a_renamed_file_with_old_comments_passes() {
    let repo = Repo::new();
    repo.write("src/old.rs", &["// issue #7: the older note", "fn a() {}"]);
    repo.commit("base");

    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    repo.commit("rename");
    // Rename detection is on by default, so a fixture that leaves the setting
    // alone proves nothing about the flag the script passes.
    repo.git_ok(&["config", "diff.renames", "false"]);

    assert_clean(&repo.run(&["HEAD~1"]));
}

#[test]
fn an_external_diff_helper_never_hides_the_added_line() {
    let repo = Repo::new();
    repo.write("src/lib.rs", &["fn a() {}"]);
    repo.commit("base");
    // An external helper replaces git's own diff, so a fixture without one
    // cannot see whether the script refuses it.
    repo.git_ok(&["config", "diff.external", "/usr/bin/true"]);
    let added = ["fn a() {}", "// issue #12: keep the ordering"];
    repo.write("src/lib.rs", &added);
    repo.commit("head");

    assert_one_hit(
        &repo.run(&["HEAD~1"]),
        &format!(
            "src/lib.rs:{}: {ISSUE_NUMBER}: // issue #12: keep the ordering",
            line_of(&added, "#12")
        ),
    );
}

#[test]
fn a_textconv_driver_never_hides_the_added_line() {
    let repo = Repo::new();
    repo.write(".gitattributes", &["*.rs diff=custom"]);
    repo.write("src/lib.rs", &["fn a() {}"]);
    repo.commit("base");
    // The driver rewrites the number the rules match, so a fixture without one
    // cannot see whether the script reads the file or the driver's rendering.
    repo.git_ok(&[
        "config",
        "diff.custom.textconv",
        "/usr/bin/sed -e s/#[0-9][0-9]*/XX/",
    ]);
    let added = ["fn a() {}", "// issue #12: keep the ordering"];
    repo.write("src/lib.rs", &added);
    repo.commit("head");

    assert_one_hit(
        &repo.run(&["HEAD~1"]),
        &format!(
            "src/lib.rs:{}: {ISSUE_NUMBER}: // issue #12: keep the ordering",
            line_of(&added, "#12")
        ),
    );
}

#[test]
fn the_check_runs_from_a_subdirectory() {
    let repo = Repo::new();
    repo.write("crates/a/src/lib.rs", &["fn a() {}"]);
    repo.write("crates/b/src/lib.rs", &["fn b() {}"]);
    repo.commit("base");

    repo.write("crates/a/src/lib.rs", &["fn a() {}", "fn a2() {}"]);
    let offending = ["fn b() {}", "// issue #12: the offending line"];
    repo.write("crates/b/src/lib.rs", &offending);
    repo.commit("head");

    assert_one_hit(
        &repo.run_in("crates/a", &["HEAD~1"]),
        &format!(
            "crates/b/src/lib.rs:{}: {ISSUE_NUMBER}: // issue #12: the offending line",
            line_of(&offending, "#12")
        ),
    );
}

#[test]
fn a_bad_base_is_an_error_not_a_pass() {
    let repo = Repo::new();
    repo.write("src/lib.rs", &["fn a() {}"]);
    repo.commit("base");
    repo.write("src/lib.rs", &["fn a() {}", "fn b() {}"]);
    repo.commit("head");

    let out = repo.run(&[]);
    assert_eq!(status(&out), 2);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), format!("comment-check: {USAGE}\n"));

    let out = repo.run(&["no-such-ref"]);
    assert_eq!(status(&out), 2);
    assert_eq!(stdout(&out), "");
    assert!(
        stderr(&out).contains("no-such-ref"),
        "stderr names the ref: {}",
        stderr(&out)
    );

    assert_clean(&repo.run(&["HEAD"]));
}

#[test]
fn only_rust_files_are_checked() {
    let repo = Repo::new();
    repo.write("docs/notes.md", &["notes"]);
    repo.write("scripts/x.py", &["x = 1"]);
    repo.write("README.md", &["readme"]);
    repo.commit("base");

    repo.write("docs/notes.md", &["notes", "// issue #12: not a Rust file"]);
    repo.write("scripts/x.py", &["x = 1", "# issue #12: not a Rust file"]);
    repo.write("README.md", &["readme", "T3 (issue #12) in prose"]);
    repo.commit("head");

    assert_clean(&repo.run(&["HEAD~1"]));

    let (out, head) = added_to_lib(&["// issue #12: a Rust file"]);
    assert_one_hit(
        &out,
        &format!(
            "src/lib.rs:{}: {ISSUE_NUMBER}: // issue #12: a Rust file",
            line_of(&head, "#12")
        ),
    );
}
