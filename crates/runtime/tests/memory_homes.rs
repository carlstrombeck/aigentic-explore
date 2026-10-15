//! A thread's memory homes reach the policy it runs under (phase 4 step 9):
//! the person's folder and the workspace's are refused as write targets,
//! and a thread that has neither gets a policy with no homes.

use std::fs;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use aigentic_core::{
    AgentId, Capabilities, CompletionRequest, Message, Provider, ProviderEvent, RiskClass, ToolCall,
};
use aigentic_log::ThreadLog;
use aigentic_policy::{MEMORY_HOME_RULE, Outcome, Policy};
use aigentic_runtime::project::{MEMORY_DIR, person_memory_dir};
use aigentic_runtime::{GlobalLayer, Layers, Runtime, WorkspaceLayer};
use aigentic_tools::ToolRegistry;
use futures_core::Stream;
use serde_json::json;

/// Enough of a provider to construct a runtime; no turn is ever run.
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
