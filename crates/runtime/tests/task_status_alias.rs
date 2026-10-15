//! A checklist row may spell its state `status`, which models mix into one
//! call, so both spellings reach the same field. The schema the model reads
//! advertises `state` only.

mod common;

use aigentic_core::{ContentBlock, Event, EventKind, ProviderEvent, ToolCall};
use aigentic_log::{AssistantMessagePayload, ToolResultPayload};
use aigentic_runtime::harness_tools::{Checklist, open_checklist};
use common::{done, harness, steve};
use serde_json::{Value, json};

/// Three steps in one call: the first spells `state`, the other two
/// `status`, as a model does when it mixes the two within a call.
fn rows() -> Value {
    json!([
        {"text": "read", "state": "done"},
        {"text": "write", "status": "done"},
        {"text": "gate", "status": "active"},
    ])
}

fn update_tasks(id: &str, tasks: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "update_tasks".into(),
        args: json!({"tasks": tasks}),
    }
}

/// The state a fixture row asks for, under either spelling: what a row
/// means is what the expectations count.
fn row_state(row: &Value) -> &str {
    row["state"]
        .as_str()
        .or_else(|| row["status"].as_str())
        .expect("a fixture row asks for a state")
}

/// Every `update_tasks` call in the log, as its assistant message holds it.
fn task_calls(events: &[Event]) -> Vec<ToolCall> {
    events
        .iter()
        .filter_map(|e| serde_json::from_value::<AssistantMessagePayload>(e.payload.clone()).ok())
        .flat_map(|p| p.blocks)
        .filter_map(|b| match b {
            ContentBlock::ToolCall(c) if c.name == "update_tasks" => Some(c),
            _ => None,
        })
        .collect()
}

/// The checklist the open turn carries, as the runtime reads it during
/// that turn: `open_checklist` over the log up to the `turn_ended` event
/// that closes the turn, since a closed turn has no checklist left.
fn checklist_of(events: &[Event]) -> Option<Checklist> {
    let end = events
        .iter()
        .rposition(|e| e.kind == EventKind::TurnEnded)
        .expect("the turn ended");
    open_checklist(&events[..end])
}

/// Every tool result in the log, in log order.
fn tool_results(events: &[Event]) -> Vec<ToolResultPayload> {
    events
        .iter()
        .filter(|e| e.kind == EventKind::ToolResult)
        .filter_map(|e| serde_json::from_value::<ToolResultPayload>(e.payload.clone()).ok())
        .collect()
}

#[tokio::test]
async fn a_call_mixing_the_two_spellings_records_each_row_and_keeps_the_log() {
    // The call's arguments, exactly as the model sends them.
    let sent = json!({ "tasks": rows() });
    let call = update_tasks("u1", sent["tasks"].clone());
    let script = vec![
        vec![ProviderEvent::ToolCall(call.clone()), done("stop")],
        vec![ProviderEvent::TextDelta("done".into()), done("stop")],
    ];
    let mut h = harness(script, None);
    let outcome = h
        .runtime
        .run_turn(steve(), vec![ContentBlock::Text("go".into())], &mut |_| {})
        .await
        .unwrap();
    assert_eq!(outcome.reason, "done", "the turn runs past the call");
    let events = h.runtime.log().read_all().unwrap();
    let fixture = sent["tasks"].as_array().unwrap();

    // The call is answered once, with the count its rows ask for: a
    // refused call returns an error and is retried, not answered.
    let results = tool_results(&events);
    let answered: Vec<&ToolResultPayload> =
        results.iter().filter(|r| r.result.id == call.id).collect();
    assert_eq!(answered.len(), 1, "one result for the call");
    assert!(!answered[0].result.is_error, "{:?}", answered[0].result);
    let done_rows = fixture.iter().filter(|r| row_state(r) == "done").count();
    assert_eq!(
        answered[0].result.content,
        format!("tasks: {done_rows} of {} done", fixture.len())
    );

    // The log holds the call's arguments as they were sent: a `status`
    // row is not rewritten into a `state` one on the way to the file.
    let logged = task_calls(&events);
    assert_eq!(logged.len(), 1, "the log holds the one call, no retry");
    let sent_json = serde_json::to_string(&sent).unwrap();
    assert_eq!(serde_json::to_string(&logged[0].args).unwrap(), sent_json);
    let file = std::fs::read_to_string(h.runtime.log().path()).unwrap();
    assert!(
        file.contains(&sent_json),
        "the log's own bytes hold the arguments as sent"
    );

    // The checklist counts every row, whichever spelling it used, and
    // leaves `since` counting the results that landed after its own.
    let checklist = checklist_of(&events).expect("the call set a checklist");
    assert_eq!(checklist.total, fixture.len());
    assert_eq!(checklist.done, done_rows);
    assert_eq!(
        checklist.active.as_deref(),
        fixture
            .iter()
            .find(|r| row_state(r) == "active")
            .map(|r| r["text"].as_str().unwrap())
    );
    let later = results
        .iter()
        .skip_while(|r| r.result.id != call.id)
        .skip(1)
        .count();
    assert_eq!(checklist.since, later);

    // Two request prompts for two scripted replies, so the accepted call
    // cost no turn.
    assert_eq!(h.seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_row_with_both_spellings_or_another_unknown_key_is_invalid_arguments() {
    let both = update_tasks(
        "b1",
        json!([{"text": "one", "state": "done", "status": "pending"}]),
    );
    let unknown = update_tasks("b2", json!([{"text": "one", "stat": "done"}]));
    let script = vec![
        vec![ProviderEvent::ToolCall(both.clone()), done("stop")],
        vec![ProviderEvent::ToolCall(unknown.clone()), done("stop")],
        vec![ProviderEvent::TextDelta("done".into()), done("stop")],
    ];
    let mut h = harness(script, None);
    h.runtime
        .run_turn(steve(), vec![ContentBlock::Text("go".into())], &mut |_| {})
        .await
        .unwrap();
    let events = h.runtime.log().read_all().unwrap();

    let results = tool_results(&events);
    let content = |id: &str| -> String {
        let answered: Vec<&ToolResultPayload> =
            results.iter().filter(|r| r.result.id == id).collect();
        assert_eq!(answered.len(), 1, "one result for the call {id}");
        assert!(answered[0].result.is_error, "{} is refused", id);
        answered[0].result.content.clone()
    };
    let in_both = content(&both.id);
    assert!(in_both.starts_with("invalid arguments:"), "{in_both}");
    assert!(
        in_both.contains("state") && in_both.contains("status"),
        "a row that spells both is refused naming both: {in_both}"
    );
    let in_unknown = content(&unknown.id);
    assert!(in_unknown.starts_with("invalid arguments:"), "{in_unknown}");
    assert!(
        in_unknown.contains("stat"),
        "an unknown key is refused naming it: {in_unknown}"
    );

    // The row errors are the turn's only results: neither refusal wrote a
    // checklist, and no third call was made.
    assert_eq!(results.len(), 2);
    assert_eq!(checklist_of(&events), None);
}

#[tokio::test]
async fn the_schema_the_model_receives_still_advertises_state_only() {
    let h = harness(vec![], None);
    let spec = h
        .runtime
        .tool_specs()
        .into_iter()
        .find(|s| s.name == "update_tasks")
        .expect("the runtime always offers update_tasks");
    let row = &spec.schema["properties"]["tasks"]["items"];
    let keys: Vec<&str> = row["properties"]
        .as_object()
        .expect("a row's properties")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["state", "text"]);
    assert_eq!(row["required"], json!(["text", "state"]));
    assert_eq!(
        row["properties"]["state"]["enum"],
        json!(["pending", "active", "done"])
    );
    let rendered = serde_json::to_string(&spec).unwrap();
    assert!(
        !rendered.contains("status"),
        "the schema mentions no other spelling: {rendered}"
    );
}
