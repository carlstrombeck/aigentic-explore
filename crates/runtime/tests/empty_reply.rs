//! A reply with no text and no tool call answered nothing, so it does not
//! end the turn as `done`: the model is nudged once, and a second empty
//! reply ends the turn `empty_reply`. Reasoning models do this: the last
//! message of a turn can hold reasoning and nothing else.
//!
//! Expected values come from the runtime's own constants and the log it
//! wrote.

use aigentic_core::{ContentBlock, Event, EventKind, Message, ProviderBlob, ProviderEvent, Role};
use aigentic_log::{ReplyNudgedPayload, TurnEndedPayload};
use aigentic_runtime::{EMPTY_REPLY, EMPTY_REPLY_NUDGE};
use serde_json::json;

mod common;
use common::{done, harness, steve};

/// What a reasoning model streams when it thinks and then says nothing.
fn reasoning_only() -> Vec<ProviderEvent> {
    vec![
        ProviderEvent::Blob(ProviderBlob {
            provider: "openai_compat".into(),
            data: json!({"reasoning_content": "Item 5 is confirmed."}),
        }),
        done("stop"),
    ]
}

fn nudges(events: &[Event]) -> Vec<ReplyNudgedPayload> {
    events
        .iter()
        .filter(|e| e.kind == EventKind::ReplyNudged)
        .map(|e| serde_json::from_value(e.payload.clone()).expect("a nudge payload"))
        .collect()
}

fn turn_ended(events: &[Event]) -> TurnEndedPayload {
    serde_json::from_value(
        events
            .iter()
            .rev()
            .find(|e| e.kind == EventKind::TurnEnded)
            .expect("the turn ended")
            .payload
            .clone(),
    )
    .expect("a turn_ended payload")
}

fn text_of(m: &Message) -> String {
    m.blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_reasoning_only_reply_is_nudged_once_and_the_answer_after_it_ends_done() {
    let mut h = harness(
        vec![
            reasoning_only(),
            vec![ProviderEvent::TextDelta("The report.".into()), done("stop")],
        ],
        None,
    );
    let outcome = h
        .runtime
        .run_turn(
            steve(),
            vec![ContentBlock::Text("check it".into())],
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(outcome.reason, "done");

    let events = h.runtime.log().read_all().unwrap();
    let nudged = nudges(&events);
    assert_eq!(nudged.len(), 1, "{nudged:?}");
    assert_eq!(nudged[0].text, EMPTY_REPLY_NUDGE);

    // The second request ends with the nudge, as a user-role message the
    // model reads after its own empty reply.
    let seen = h.seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "one request per reply");
    let last = seen[1].last().expect("a message");
    assert_eq!(last.role, Role::User);
    assert_eq!(text_of(last), EMPTY_REPLY_NUDGE);
}

#[tokio::test]
async fn a_second_empty_reply_ends_the_turn_empty_reply_and_asks_no_third_time() {
    let mut h = harness(
        vec![
            reasoning_only(),
            // Whitespace is not text a person can read.
            vec![ProviderEvent::TextDelta(" \n".into()), done("stop")],
            vec![
                ProviderEvent::TextDelta("never asked for".into()),
                done("stop"),
            ],
        ],
        None,
    );
    let outcome = h
        .runtime
        .run_turn(
            steve(),
            vec![ContentBlock::Text("check it".into())],
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(outcome.reason, EMPTY_REPLY);

    let events = h.runtime.log().read_all().unwrap();
    assert_eq!(nudges(&events).len(), 1);
    assert_eq!(turn_ended(&events).reason, EMPTY_REPLY);
    assert_eq!(h.seen.lock().unwrap().len(), 2, "no third request");
}

#[tokio::test]
async fn the_nudge_is_per_turn_so_the_next_turn_gets_its_own() {
    let mut h = harness(
        vec![
            reasoning_only(),
            vec![ProviderEvent::TextDelta("one".into()), done("stop")],
            reasoning_only(),
            vec![ProviderEvent::TextDelta("two".into()), done("stop")],
        ],
        None,
    );
    for prompt in ["first", "second"] {
        let outcome = h
            .runtime
            .run_turn(
                steve(),
                vec![ContentBlock::Text(prompt.into())],
                &mut |_| {},
            )
            .await
            .unwrap();
        assert_eq!(outcome.reason, "done", "{prompt}");
    }
    assert_eq!(nudges(&h.runtime.log().read_all().unwrap()).len(), 2);
}
