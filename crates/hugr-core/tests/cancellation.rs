//! Phase 2 (P2-2): first-class cancellation. A `Cancel` command (driven by a
//! `UserAbort`, or steer-interrupt) aborts an in-flight op; the host confirms
//! with `OpCancelled`; the brain logs the *partial* work as a `Cancelled`
//! outcome — "N tokens then cancelled" — never an implicit gap (ARCHITECTURE
//! §6.4). These tests pin the command sequence and assert deterministic replay
//! (CLAUDE.md: determinism is testable).

mod common;

use common::*;
use hugr_core::{
    Brain, Command, DoneReason, Event, ModelDelta, OpId, OpOutcome, Record, StaticPolicy, Value,
};
use serde_json::json;

/// A policy that runs `shell` in the background (does not block the turn).
fn background_shell_policy() -> StaticPolicy {
    StaticPolicy::default().with_background(["shell".to_string()])
}

/// The partial text preserved for a cancelled model op, if any.
fn cancelled_partial(log: &[hugr_core::LogEntry], op: OpId) -> Option<Value> {
    log.iter().find_map(|e| match &e.record {
        Record::OpEnded {
            op: o,
            outcome: OpOutcome::Cancelled { partial },
            ..
        } if *o == op => Some(partial.clone()),
        _ => None,
    })
}

/// The headline P2-2 scenario: a model stream produces a few tokens, then the
/// user aborts (ESC). The host aborts the task and confirms with `OpCancelled`;
/// the brain records the partial text ("Hello, wor") as a `Cancelled` outcome
/// and ends the turn `Cancelled`.
#[test]
fn stream_some_tokens_then_cancel_records_the_partial() {
    let mut brain = Brain::with_default_policy();

    let commands = run_script(
        &mut brain,
        vec![
            user("write a poem"),
            // The model streams a couple of tokens (transport only; not logged).
            Event::ModelDelta {
                op: OpId(0),
                delta: ModelDelta::Text("Hello, ".into()),
            },
            Event::ModelDelta {
                op: OpId(0),
                delta: ModelDelta::Text("wor".into()),
            },
            // User hits ESC: a pure abort. The brain asks the host to cancel
            // every in-flight op.
            Event::UserAbort,
            // The host aborted the model task and confirms it.
            Event::OpCancelled { op: OpId(0) },
        ],
    );

    let effectful = effectful(&commands);
    assert!(
        matches!(
            effectful.as_slice(),
            [
                Command::StartModelCall { op: OpId(0), .. },
                // UserAbort → cancel the in-flight model op.
                Command::Cancel { op: OpId(0) },
                // OpCancelled → the turn is over, cancelled.
                Command::Done {
                    reason: DoneReason::Cancelled
                },
            ]
        ),
        "unexpected command sequence: {effectful:#?}"
    );

    // The partial output ("N tokens then cancelled") is preserved in the log.
    let partial = cancelled_partial(brain.state().log(), OpId(0));
    assert_eq!(partial, Some(json!("Hello, wor")));
}

/// Replay: re-feeding the identical event stream (stream N tokens, then cancel)
/// to a fresh brain yields identical commands AND an identical durable log — the
/// partial is reproduced before the cancel, deterministically (ARCHITECTURE §6.2).
#[test]
fn cancellation_replay_is_deterministic() {
    let script = || {
        vec![
            user("write a poem"),
            Event::ModelDelta {
                op: OpId(0),
                delta: ModelDelta::Text("Hello, ".into()),
            },
            Event::ModelDelta {
                op: OpId(0),
                delta: ModelDelta::Text("wor".into()),
            },
            Event::UserAbort,
            Event::OpCancelled { op: OpId(0) },
        ]
    };

    let mut a = Brain::with_default_policy();
    let commands_a = run_script(&mut a, script());

    let mut b = Brain::with_default_policy();
    let commands_b = run_script(&mut b, script());

    assert_eq!(
        commands_a, commands_b,
        "stream-then-cancel must replay to identical commands"
    );
    assert_eq!(
        a.state().log(),
        b.state().log(),
        "stream-then-cancel must replay to an identical log (partial then cancelled)"
    );

    // And the log really does contain the partial then the Cancelled outcome.
    assert_eq!(
        cancelled_partial(a.state().log(), OpId(0)),
        Some(json!("Hello, wor"))
    );
}

/// A stale terminal event racing the abort: the host aborts the task, but the
/// task had already queued its real `ModelDone` a hair earlier. The brain folds
/// the `ModelDone` first (op resolves `Ok`), then the now-stale `OpCancelled`
/// arrives — it must be a no-op (idempotent), not append a spurious `Cancelled`
/// `OpEnded` that would corrupt the log / break replay.
#[test]
fn stale_op_cancelled_after_done_is_ignored() {
    let mut brain = Brain::with_default_policy();

    let commands = run_script(
        &mut brain,
        vec![
            user("hi"),
            Event::ModelDelta {
                op: OpId(0),
                delta: ModelDelta::Text("done".into()),
            },
            // The real terminal event lands first (op completes Ok, turn ends).
            Event::ModelDone {
                op: OpId(0),
                output: text_output("done"),
                usage: usage(),
            },
            // ...then the late cancel confirmation for the same op arrives.
            Event::OpCancelled { op: OpId(0) },
        ],
    );

    let effectful = effectful(&commands);
    assert!(
        matches!(
            effectful.as_slice(),
            [
                Command::StartModelCall { op: OpId(0), .. },
                Command::Checkpoint,
                Command::Done {
                    reason: DoneReason::EndTurn
                },
                // The stale OpCancelled produced NO further command.
            ]
        ),
        "stale OpCancelled should be a no-op: {effectful:#?}"
    );

    // Exactly one OpEnded, with the Ok outcome — no spurious Cancelled entry.
    let op_ends: Vec<&OpOutcome> = brain
        .state()
        .log()
        .iter()
        .filter_map(|e| match &e.record {
            Record::OpEnded { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect();
    assert_eq!(op_ends.len(), 1);
    assert!(matches!(op_ends[0], OpOutcome::Ok));
}

/// Cancelling one background op while the model is still streaming must NOT end
/// the turn: the model op is still in flight, so the brain stays busy. Proves
/// the terminal `Done { Cancelled }` only fires once the *last* op drains.
#[test]
fn cancelling_a_background_op_mid_stream_does_not_end_the_turn() {
    let mut brain = Brain::new(Box::new(background_shell_policy()));

    let commands = run_script(
        &mut brain,
        vec![
            user("build and chat"),
            // Model kicks off a background shell (op 1); the turn resumes into a
            // second model call (op 2) — both now in flight.
            Event::ModelDone {
                op: OpId(0),
                output: tool_output("call-1", "shell", json!({ "cmd": "cargo build" })),
                usage: usage(),
            },
            // Cancel just the background shell op while the model (op 2) streams.
            Event::OpCancelled { op: OpId(1) },
            // The model finishes; with the background op gone and nothing else in
            // flight, the turn ends normally (EndTurn, not Cancelled).
            Event::ModelDone {
                op: OpId(2),
                output: text_output("All set."),
                usage: usage(),
            },
        ],
    );

    let effectful = effectful(&commands);
    assert!(
        matches!(
            effectful.as_slice(),
            [
                Command::StartModelCall { op: OpId(0), .. },
                Command::StartCapability { op: OpId(1), .. },
                Command::StartModelCall { op: OpId(2), .. },
                // No Done after the background cancel — the model op still runs.
                Command::Checkpoint,
                Command::Done {
                    reason: DoneReason::EndTurn
                },
            ]
        ),
        "unexpected command sequence: {effectful:#?}"
    );

    // The cancelled background op was logged as Cancelled (with a null partial,
    // since it produced no model text).
    assert_eq!(
        cancelled_partial(brain.state().log(), OpId(1)),
        Some(Value::Null)
    );
}
