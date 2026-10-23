//! A thread's memory homes reach the policy it runs under (phase 4 step 9):
//! the person's folder and the workspace's are refused as write targets,
//! and a thread that has neither gets a policy with no homes.

use std::fs;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use aigentic_core::{
    AgentId, Author, Capabilities, CompletionRequest, ContentBlock, EventKind, Message, Provider,
    ProviderEvent, RiskClass, ToolCall, UserId,
};
use aigentic_log::{ThreadLog, ToolResultPayload};
use aigentic_policy::{MEMORY_HOME_RULE, MEMORY_REASON, Outcome, Policy};
use aigentic_runtime::project::{MEMORY_DIR, person_memory_dir};
use aigentic_runtime::{GlobalLayer, Layers, Mode, Runtime, WorkspaceLayer};
use aigentic_tools::{DEFAULT_TIMEOUT, ToolRegistry, Workdir};
use futures_core::Stream;
use serde_json::json;

mod common;

use common::{done, scripted};

/// Enough of a provider to construct a runtime; tests that run a turn
/// script their own.
struct Quiet;

impl Provider for Quiet {
    fn complete(
        &self,
        _: &CompletionRequest<'_>,
    ) -> Pin<Box<dyn Stream<Item = ProviderEvent> + Send + '_>> {
        unimplemented!("this test never calls the model")
    }
    fn count_tokens(&self, _: &[Message]) -> u64 {
        0
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_tools: true,
            supports_images: false,
            supports_caching: false,
            supports_structured_output: false,
            max_context_tokens: 1_048_576,
        }
    }
}

/// The person's config dir, a workspace's shared dir and a project, all in
/// one temp dir.
struct Fixture {
    _dir: tempfile::TempDir,
    config: PathBuf,
    shared: PathBuf,
    project: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let config = base.join("config");
        let shared = base.join("shared");
        let project = base.join("project");
        for d in [&config, &shared, &project] {
            fs::create_dir_all(d).unwrap();
        }
        let person = person_memory_dir(&config);
        fs::create_dir_all(&person).unwrap();
        fs::write(person.join("facts.md"), "x").unwrap();
        let workspace = WorkspaceLayer::dir(&shared).join(MEMORY_DIR);
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("facts.md"), "x").unwrap();
        Self {
            _dir: dir,
            config,
            shared,
            project,
        }
    }

    /// The person's folder, as the loader is given it.
    fn person(&self) -> PathBuf {
        person_memory_dir(&self.config)
    }

    /// The layers a thread is built with. `owner` names who owns the
    /// person's folder; without one the loader gives the thread no person
    /// home.
    fn layers(&self, owner: Option<&str>, shared: Option<&Path>) -> Layers {
        let global = GlobalLayer::load(
            &self.config.join("instructions.md"),
            Some(&self.person()),
            owner,
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let workspace = Some(WorkspaceLayer::load("ws", shared).unwrap());
        Layers {
            global,
            workspace,
            project: None,
        }
    }

    /// A runtime over a thread's layers and policy, as a client builds one.
    fn runtime(&self, layers: Layers, policy: Policy) -> Runtime {
        let log = ThreadLog::open(&self.project, ulid::Ulid::generate()).unwrap();
        Runtime::new(
            Box::new(Quiet),
            ToolRegistry::empty(),
            log,
            AgentId("worker".into()),
        )
        .with_layers(layers)
        .with_policy(policy)
    }
}

fn call(tool: &str, path: &Path) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        name: tool.into(),
        args: json!({ "path": path.to_str().unwrap() }),
    }
}

#[test]
fn a_threads_memory_homes_reach_its_policy() {
    let f = Fixture::new();
    let layers = f.layers(Some("steve"), Some(&f.shared));
    let expected: Vec<PathBuf> = layers
        .extra_memory_homes()
        .into_iter()
        .filter_map(|home| layers.memory_home(home))
        .collect();
    assert_eq!(
        expected.len(),
        2,
        "the fixture thread has the person's folder and the workspace's"
    );
    let runtime = f.runtime(layers, Policy::defaults().with_root(&f.project, &f.project));
    assert_eq!(runtime.policy().memory_homes(), expected);

    for home in expected {
        for tool in ["write_file", "edit_file"] {
            let outcome = runtime.policy().decide(
                &call(tool, &home.join("facts.md")),
                RiskClass::Write,
                None,
            );
            assert!(
                matches!(&outcome, Outcome::Deny { rule, .. } if rule == MEMORY_HOME_RULE),
                "{tool} under {} is a write under a home, got {outcome:?}",
                home.display()
            );
        }
    }
}

#[test]
fn a_thread_without_a_home_gets_an_empty_list() {
    let f = Fixture::new();
    let layers = f.layers(None, None);
    let runtime = f.runtime(layers, Policy::defaults().with_root(&f.project, &f.project));
    assert!(runtime.policy().memory_homes().is_empty());

    let facts = f.person().join("facts.md");
    let without = Policy::defaults().with_root(&f.project, &f.project);
    for tool in ["write_file", "edit_file"] {
        let here = runtime
            .policy()
            .decide(&call(tool, &facts), RiskClass::Write, None);
        assert!(
            !matches!(&here, Outcome::Deny { rule, .. } if rule == MEMORY_HOME_RULE),
            "{tool} with no home is not the memory deny, got {here:?}"
        );
        assert_eq!(
            here,
            without.decide(&call(tool, &facts), RiskClass::Write, None),
            "{tool} with no home decides as a policy without homes"
        );
        assert!(
            matches!(here, Outcome::AskBoundary { .. }),
            "{tool} outside the root asks about the boundary, got {here:?}"
        );
    }
}

/// Runs one `bash` call, then a stop, in `mode`, over the fixture's thread
/// layers (both homes) and its project as root, and answers with the text
/// the call's result carries. The built-in tools are registered, so the
/// command really changes directory and writes.
async fn run_bash(f: &Fixture, command: &str, mode: Mode) -> String {
    let (provider, _seen) = scripted(vec![
        vec![
            ProviderEvent::ToolCall(ToolCall {
                id: "c1".into(),
                name: "bash".into(),
                args: json!({ "command": command }),
            }),
            done("tool_use"),
        ],
        vec![done("stop")],
    ]);
    let log = ThreadLog::open(&f.project, ulid::Ulid::generate()).unwrap();
    let mut runtime = Runtime::new(
        provider,
        ToolRegistry::builtin(Workdir::new(&f.project), DEFAULT_TIMEOUT),
        log,
        AgentId("worker".into()),
    )
    .with_layers(f.layers(Some("steve"), Some(&f.shared)))
    .with_policy(Policy::defaults().with_root(&f.project, &f.project));
    runtime.set_mode(mode);
    runtime
        .run_turn(
            Author::User(UserId("steve".into())),
            vec![ContentBlock::Text("go".into())],
            &mut |_| {},
        )
        .await
        .unwrap();
    for e in runtime.log().read_all().unwrap() {
        if e.kind != EventKind::ToolResult {
            continue;
        }
        let p: ToolResultPayload = serde_json::from_value(e.payload.clone()).unwrap();
        if p.result.id == "c1" {
            return p.result.content;
        }
    }
    panic!("no tool result for c1");
}

/// A command that changes directory into a home and writes there, in the
/// same call, is refused: the check reads where the words after the `cd`
/// write, and `Mode::Auto`, which answers every ask, would otherwise let
/// the write land.
#[tokio::test]
async fn a_cd_into_a_home_and_a_write_in_one_call_is_refused() {
    let f = Fixture::new();
    let home = f.person();
    let text = run_bash(
        &f,
        &format!("cd {} && printf x > note.md", home.display()),
        Mode::Auto,
    )
    .await;
    assert!(
        text.contains(MEMORY_HOME_RULE) && text.contains(MEMORY_REASON),
        "the refusal names the homes' rule and reason, got {text:?}"
    );
    assert!(
        !home.join("note.md").exists(),
        "nothing was written under {}",
        home.display()
    );
}

/// The same shape outside every home runs under the mode that answers an
/// ask, so the refusal above is the home the words name and not the `cd`.
#[tokio::test]
async fn a_cd_and_a_write_outside_every_home_still_runs() {
    let f = Fixture::new();
    let here = f.project.join("sub");
    fs::create_dir_all(&here).unwrap();
    let text = run_bash(
        &f,
        &format!("cd {} && printf x > facts.md", here.display()),
        Mode::Auto,
    )
    .await;
    assert!(
        !text.contains(MEMORY_HOME_RULE),
        "no home is named, got {text:?}"
    );
    assert!(
        here.join("facts.md").exists(),
        "the write landed in {}",
        here.display()
    );
}
