//! A `bash` read of a memory home stays allowed: the write-target reader
//! is what the home check consults, so the `bash` allow-list's own
//! decisions are untouched, and only a command that writes under a home
//! is refused.

use std::fs;

use aigentic_core::{RiskClass, ToolCall};
use aigentic_policy::{
    MEMORY_HOME_RULE, Outcome, Policy, canonicalise, default_bash_allow, write_targets,
};
use serde_json::json;

fn call(command: &str) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        name: "bash".into(),
        args: json!({ "command": command }),
    }
}

#[test]
fn a_read_of_a_home_is_allowed_and_a_write_is_denied() {
    let dir = tempfile::tempdir().unwrap();
    let project = canonicalise(dir.path());
    let home = project.join("memory");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("facts.md"), "x").unwrap();
    let policy = Policy::defaults()
        .with_root(&project, &project)
        .with_memory_homes([home.clone()]);
    let facts = home.join("facts.md");

    let read = policy.decide(
        &call(&format!("cat {}", facts.display())),
        RiskClass::Write,
        None,
    );
    let pattern = default_bash_allow()
        .into_iter()
        .find(|p| p == "cat")
        .expect("`cat` is a default allow pattern");
    assert!(
        matches!(&read, Outcome::Allow { rule } if rule.contains(&pattern)),
        "the allow-list still decides a read of a home, got {read:?}"
    );
    assert_eq!(
        write_targets(&format!("cat {}", facts.display())),
        Vec::<String>::new(),
        "a read names no write target"
    );

    for command in [
        format!("sed -i s/a/b/ {}", facts.display()),
        format!("tee {}", facts.display()),
    ] {
        let outcome = policy.decide(&call(&command), RiskClass::Write, None);
        assert!(
            matches!(&outcome, Outcome::Deny { rule, .. } if rule == MEMORY_HOME_RULE),
            "{command} writes under the home, got {outcome:?}"
        );
    }
}
