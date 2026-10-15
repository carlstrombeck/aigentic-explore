//! The memory homes: the person's and each workspace's memory folder are
//! written by extraction, never by the model, and no rule may allow one.
//!
//! Every row is checked against a policy with the homes and against the
//! same policy without them, so the homes account for exactly the
//! refusals the rows name.

use std::fs;
use std::path::{Path, PathBuf};

use aigentic_core::{RiskClass, ToolCall};
use aigentic_policy::{
    Decision, MEMORY_HOME_RULE, MEMORY_REASON, Outcome, Policy, Rule, canonicalise,
};
use serde_json::json;

/// One temp dir holding a project, the person's memory folder and a shared
/// workspace's, with a symlink from the project into the person's.
struct Fixture {
    _dir: tempfile::TempDir,
    /// The config dir: the person's folder and the workspace tree sit in
    /// it, and its siblings share a name prefix with the person's folder.
    base: PathBuf,
    project: PathBuf,
    person: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = canonicalise(dir.path());
        let project = base.join("project");
        let person = base.join("person-memory");
        let workspace = base.join("shared/workspace/memory");
        for d in [&project, &person, &workspace] {
            fs::create_dir_all(d).unwrap();
        }
        for h in [&person, &workspace] {
            fs::write(h.join("facts.md"), "x").unwrap();
        }
        fs::create_dir_all(base.join("memoryX")).unwrap();
        fs::write(base.join("memory-notes.md"), "x").unwrap();
        std::os::unix::fs::symlink(&person, project.join("link")).unwrap();
        Self {
            _dir: dir,
            base,
            project,
            person,
            workspace,
        }
    }

    fn homes(&self) -> [PathBuf; 2] {
        [self.person.clone(), self.workspace.clone()]
    }

    /// The default table over the project, bounded by `boundary`, with the
    /// homes unless `homes` is false.
    fn policy(&self, homes: bool, boundary: &[PathBuf]) -> Policy {
        let p = Policy::defaults()
            .with_root(&self.project, &self.project)
            .with_boundary_roots(boundary.to_vec());
        if homes {
            p.with_memory_homes(self.homes())
        } else {
            p
        }
    }
}

fn call(tool: &str, path: &Path) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        name: tool.into(),
        args: json!({ "path": path.to_str().unwrap() }),
    }
}

fn class(tool: &str) -> RiskClass {
    match tool {
        "write_file" | "edit_file" => RiskClass::Write,
        _ => RiskClass::Read,
    }
}

/// The refusal the homes justify: the memory-home rule, naming the home the
/// call would write under.
fn denied_in(home: &Path) -> Outcome {
    Outcome::Deny {
        rule: MEMORY_HOME_RULE.to_owned(),
        reason: format!("{MEMORY_REASON}: {}", canonicalise(home).display()),
    }
}

/// Whether a policy refused the call as a write under a memory home.
fn is_memory_deny(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Deny { rule, .. } if rule == MEMORY_HOME_RULE)
}

#[test]
fn writing_a_file_under_a_memory_home_is_denied() {
    let f = Fixture::new();
    let p = f.policy(true, &[]);
    for home in f.homes() {
        for path in [home.clone(), home.join("sub"), home.join("facts.md")] {
            for tool in ["write_file", "edit_file"] {
                assert_eq!(
                    p.decide(&call(tool, &path), class(tool), None),
                    denied_in(&home),
                    "{tool} on {}",
                    path.display()
                );
            }
        }
    }
    // The refusal does not depend on where the thread's working directory
    // is: an absolute path under a home is refused, not asked about.
    let absolute = call("write_file", &f.person.join("facts.md"));
    assert_eq!(
        p.decide(&absolute, RiskClass::Write, Some(&f.base)),
        denied_in(&f.person)
    );
}

#[test]
fn reading_a_memory_home_is_not_denied() {
    let f = Fixture::new();
    let p = f.policy(true, &[]);
    for path in [f.person.clone(), f.person.join("facts.md")] {
        for tool in ["read_file", "list_dir", "grep"] {
            let outcome = p.decide(&call(tool, &path), class(tool), None);
            assert!(
                !is_memory_deny(&outcome),
                "{tool} on {} is a read, got {outcome:?}",
                path.display()
            );
            assert_eq!(
                outcome,
                f.policy(false, &[])
                    .decide(&call(tool, &path), class(tool), None),
                "the homes change nothing for {tool} on {}",
                path.display()
            );
        }
    }
}

#[test]
fn a_rule_that_would_allow_the_write_is_outranked() {
    let f = Fixture::new();
    let facts = f.person.join("facts.md");
    for tool in ["write_file", "edit_file"] {
        let row = Rule::tool_path(
            tool,
            &format!("{}/", f.person.display()),
            Decision::Allow,
            "",
        );
        let allowed_with = row.name();
        let policy = |homes: bool| {
            let p =
                Policy::configured(vec![row.clone()], None).with_boundary_roots([f.base.clone()]);
            if homes {
                p.with_memory_homes(f.homes())
            } else {
                p
            }
        };
        assert_eq!(
            policy(false).decide(&call(tool, &facts), RiskClass::Write, None),
            Outcome::Allow { rule: allowed_with },
            "the rule's prefix matches the path, so it decides alone"
        );
        assert_eq!(
            policy(true).decide(&call(tool, &facts), RiskClass::Write, None),
            denied_in(&f.person),
            "the home outranks the rule for {tool}"
        );
    }
}

#[test]
fn only_a_path_inside_a_home_is_refused() {
    let f = Fixture::new();
    let p = f.policy(true, &[]);
    let without = f.policy(false, &[]);
    let call_link = call("write_file", &f.project.join("link/facts.md"));
    assert_eq!(
        p.decide(&call_link, RiskClass::Write, None),
        denied_in(&f.person),
        "a symlink into a home is still a write under it"
    );
    let without_link = without.decide(&call_link, RiskClass::Write, None);
    assert!(
        matches!(
            &without_link,
            Outcome::AskBoundary { path: Some(p), .. }
                if *p == canonicalise(&f.person.join("facts.md"))
        ),
        "without the homes the same canonical file is what the boundary asks about, got {without_link:?}"
    );
    let siblings = [
        f.base.join("memory-notes.md"),
        f.base.join("memoryX/f"),
        f.project.join("memory/facts.md"),
        f.person.join("../memoryX/facts.md"),
    ];
    for path in siblings {
        let call = call("write_file", &path);
        for policy in [&p, &without] {
            let outcome = policy.decide(&call, RiskClass::Write, None);
            assert!(
                !is_memory_deny(&outcome),
                "{} is not a memory home, got {outcome:?}",
                path.display()
            );
        }
        assert_eq!(
            p.decide(&call, RiskClass::Write, None),
            without.decide(&call, RiskClass::Write, None),
            "the homes change nothing for {}",
            path.display()
        );
    }
}

/// A `bash` call as the loop hands it to policy: the command word and its
/// words, with an explicit working directory in `decide`.
fn bash(command: &str) -> ToolCall {
    ToolCall {
        id: "c2".into(),
        name: "bash".into(),
        args: json!({ "command": command }),
    }
}

/// Whether a policy allowed the call.
fn is_allow(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Allow { .. })
}

#[test]
fn a_bash_write_target_under_a_home_is_denied() {
    let f = Fixture::new();
    let p = Policy::defaults()
        .with_root(&f.project, &f.project)
        .with_memory_homes(f.homes());
    let without = Policy::defaults().with_root(&f.project, &f.project);
    for home in f.homes() {
        let facts = home.join("facts.md");
        let rows = [
            format!("tee {facts}", facts = facts.display()),
            format!("echo x | tee {facts}", facts = facts.display()),
            format!("sed -i s/a/b/ {facts}", facts = facts.display()),
            format!("cp a {facts}", facts = facts.display()),
            format!("mv a {facts}", facts = facts.display()),
            format!("echo x >> {facts}", facts = facts.display()),
            // The directory a `cd` word names is where the words after it
            // write, so a relative target there is refused; the reader
            // moves the directory across `&&` and `&` alike.
            format!("cd {home} && echo x >> facts.md", home = home.display()),
            format!("cd {home} & printf x > facts.md", home = home.display()),
        ];
        for command in rows {
            assert_eq!(
                p.decide(&bash(&command), RiskClass::Write, None),
                denied_in(&home),
                "{command}"
            );
            assert!(
                !is_memory_deny(&without.decide(&bash(&command), RiskClass::Write, None)),
                "without the homes {command} is not this deny"
            );
        }
    }
    // The fixture's symlink reaches the person's folder from inside the
    // project, so the write is still under that home.
    assert_eq!(
        p.decide(&bash("echo x >> link/facts.md"), RiskClass::Write, None),
        denied_in(&f.person)
    );
    // A read of a home and a write outside every home are what they were:
    // the `bash` allow-list allows the first, the boundary asks about the
    // second.
    let facts = f.person.join("facts.md");
    let read = bash(&format!("cat {}", facts.display()));
    assert!(
        is_allow(&p.decide(&read, RiskClass::Write, None)),
        "the `bash` allow rule still allows a read of a home"
    );
    let outside = bash("echo x > /tmp/out");
    let asked = p.decide(&outside, RiskClass::Write, None);
    assert!(
        matches!(asked, Outcome::Ask { .. }),
        "a write outside every home still asks, got {asked:?}"
    );
    assert_eq!(
        asked,
        without.decide(&outside, RiskClass::Write, None),
        "a write outside every home is unchanged"
    );
}

#[test]
fn a_tilde_write_target_is_expanded_against_home() {
    let f = Fixture::new();
    // `HOME` is process-wide and `set_var` is unsafe; this test is the only
    // reader of it in the crate, and the window closes before any assertion
    // so a failure cannot leave it set.
    let previous = std::env::var_os("HOME");
    let p = Policy::defaults()
        .with_root(&f.project, &f.project)
        .with_memory_homes(f.homes());
    let without = Policy::defaults().with_root(&f.project, &f.project);
    let tilde = "~/person-memory/facts.md";
    let decisions = {
        unsafe { std::env::set_var("HOME", &f.base) };
        let rows = [
            p.decide(
                &bash(&format!("cat {facts} {tilde}", facts = f.person.display())),
                RiskClass::Write,
                None,
            ),
            p.decide(
                &bash(&format!("tee {facts} {tilde}", facts = f.person.display())),
                RiskClass::Write,
                None,
            ),
            without.decide(&bash(&format!("tee {tilde}")), RiskClass::Write, None),
            without.decide(
                &bash(&format!("tee {}", f.person.join("facts.md").display())),
                RiskClass::Write,
                None,
            ),
        ];
        match previous.clone() {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        rows
    };
    assert!(
        is_allow(&decisions[0]),
        "reading through `~` is a read, got {:?}",
        decisions[0]
    );
    assert_eq!(
        decisions[1],
        denied_in(&f.person),
        "`~` resolved against HOME is a write under the home"
    );
    assert!(
        !is_memory_deny(&decisions[2]),
        "without the homes the same `~` command is not this deny"
    );
    assert_eq!(
        decisions[2], decisions[3],
        "without the homes, `~` and the path it names decide alike"
    );
}

/// A home is a folder, not a prefix of a name: a sibling whose name starts
/// the same is a different path and no write under it is refused as a home
/// write.
#[test]
fn a_sibling_whose_name_starts_like_a_home_is_not_inside_it() {
    let f = Fixture::new();
    let home = f.base.join("memory");
    fs::create_dir_all(&home).unwrap();
    let p = Policy::defaults()
        .with_root(&f.project, &f.project)
        .with_memory_homes([home.clone()]);
    let without = Policy::defaults().with_root(&f.project, &f.project);
    assert_eq!(
        p.decide(
            &call("write_file", &home.join("facts.md")),
            RiskClass::Write,
            None
        ),
        denied_in(&home),
        "a file in the folder the home names is under it"
    );
    for path in [f.base.join("memoryX/f"), f.base.join("memory-notes.md")] {
        let call = call("write_file", &path);
        let here = p.decide(&call, RiskClass::Write, None);
        assert!(
            !is_memory_deny(&here),
            "{} is beside {}, not under it, got {here:?}",
            path.display(),
            home.display()
        );
        assert_eq!(
            here,
            without.decide(&call, RiskClass::Write, None),
            "the home changes nothing for {}",
            path.display()
        );
    }
}

#[test]
fn the_boundary_still_decides_reads_around_a_home() {
    let f = Fixture::new();
    for path in [f.person.clone(), f.person.join("facts.md")] {
        let bounded = f.policy(true, std::slice::from_ref(&f.base));
        let unbounded = f.policy(true, &[]);
        let pair = |p: &Policy| p.decide(&call("read_file", &path), RiskClass::Read, None);
        assert!(matches!(pair(&bounded), Outcome::Allow { .. }));
        assert!(matches!(pair(&unbounded), Outcome::AskBoundary { .. }));
        assert_eq!(
            pair(&bounded),
            pair(&f.policy(false, std::slice::from_ref(&f.base)))
        );
        assert_eq!(pair(&unbounded), pair(&f.policy(false, &[])));
    }
    let bounded = f.policy(true, std::slice::from_ref(&f.base));
    let listed = bounded.decide(&call("list_dir", &f.person), RiskClass::Read, None);
    assert!(
        !is_memory_deny(&listed),
        "listing the home itself is a read, got {listed:?}"
    );
}
