//! Externalized permission policy (DESIGN §5.4).
//!
//! The brain emits a `RequestPermission` command; an external, pluggable
//! `Policy` decides allow/deny. The brain's loop is identical whether the host
//! prompts a human, consults an allowlist, or auto-approves.

use std::sync::Arc;

use async_trait::async_trait;
use hugr_core::{
    ContentPart, ContextBlock, Decision, ModelRequest, ModelSelector, OpId, PermissionRequest,
    Role, SamplingParams, Value,
};
use serde_json::json;
use tokio::sync::mpsc;

use crate::model::{ModelAdapter, ModelSink};

/// Decides whether a gated capability invocation may proceed.
#[async_trait]
pub trait Policy: Send + Sync {
    async fn decide(&self, request: &PermissionRequest) -> Decision;
}

/// Approves everything (the `-y/--yes` mode). Decisions still flow through the
/// brain as events, so they are recorded in the trace.
pub struct AllowAll;

#[async_trait]
impl Policy for AllowAll {
    async fn decide(&self, _request: &PermissionRequest) -> Decision {
        Decision::Allow
    }
}

/// Denies everything (useful for headless/locked-down runs).
pub struct DenyAll;

#[async_trait]
impl Policy for DenyAll {
    async fn decide(&self, _request: &PermissionRequest) -> Decision {
        Decision::Deny {
            reason: "denied by policy".to_string(),
        }
    }
}

/// Host-side auto-approval policy. For each gated action it asks a small-tier
/// judge model for a `{ "safe": bool, "reason": string }` verdict, then returns
/// `Allow` or `Deny { reason }`. The verdict itself is fed back to the brain as
/// a recorded `PermissionDecision` event by the engine, so replay reuses the
/// recorded decision and never calls this judge again.
pub struct AutoApprove {
    judge: Arc<dyn ModelAdapter>,
    selector: ModelSelector,
}

impl AutoApprove {
    /// Construct an auto-approve policy backed by the given model adapter. The
    /// CLI passes the configured `small` tier adapter here.
    pub fn new(judge: Arc<dyn ModelAdapter>) -> Self {
        Self {
            judge,
            selector: ModelSelector::named("small"),
        }
    }

    pub fn with_selector(mut self, selector: ModelSelector) -> Self {
        self.selector = selector;
        self
    }
}

#[async_trait]
impl Policy for AutoApprove {
    async fn decide(&self, request: &PermissionRequest) -> Decision {
        let model_request = judge_request(request, &self.selector);
        let (tx, _rx) = mpsc::unbounded_channel();
        let sink = ModelSink::new(OpId(u64::MAX), tx);
        match self.judge.call(model_request, &sink).await {
            Ok((output, _usage)) => parse_judge_decision(&output.text),
            Err(err) => Decision::Deny {
                reason: format!("auto-approve judge failed: {err}"),
            },
        }
    }
}

fn judge_request(request: &PermissionRequest, selector: &ModelSelector) -> ModelRequest {
    let system = concat!(
        "You are Hugr's permission judge. Decide whether a requested tool ",
        "invocation is safe to run without asking a human. Return only JSON ",
        "with shape {\"safe\":true|false,\"reason\":\"short reason\"}. ",
        "Allow benign bounded actions. Deny destructive filesystem changes, ",
        "credential access, secret exfiltration, broad deletes, privilege ",
        "changes, and unclear high-risk commands."
    );
    let user = json!({
        "tier": selector,
        "capability": request.capability,
        "args": request.args,
        "instruction": "Classify this action."
    });
    ModelRequest::new(
        vec![
            ContextBlock::new(Role::System, vec![ContentPart::Text(system.to_string())]),
            ContextBlock::new(Role::User, vec![ContentPart::Text(user.to_string())]),
        ],
        Vec::new(),
        SamplingParams::new()
            .with_temperature(0.0)
            .with_max_tokens(128),
    )
}

fn parse_judge_decision(text: &str) -> Decision {
    let parsed = serde_json::from_str::<Value>(text)
        .ok()
        .or_else(|| extract_json_object(text).and_then(|s| serde_json::from_str(s).ok()));
    let Some(value) = parsed else {
        return Decision::Deny {
            reason: "auto-approve judge returned an unparsable verdict".to_string(),
        };
    };
    let safe = value.get("safe").and_then(Value::as_bool).unwrap_or(false);
    let reason = value
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or(if safe {
            "auto-approve judge allowed the action"
        } else {
            "auto-approve judge denied the action"
        })
        .to_string();
    if safe {
        Decision::Allow
    } else {
        Decision::Deny { reason }
    }
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (start <= end).then_some(&text[start..=end])
}

/// Prompts the user on the terminal for each gated capability (`y/N`).
pub struct Interactive;

#[async_trait]
impl Policy for Interactive {
    async fn decide(&self, request: &PermissionRequest) -> Decision {
        let capability = request.capability.clone();
        let args = request.args.clone();
        // Reading stdin is blocking; keep it off the async runtime threads.
        let answer = tokio::task::spawn_blocking(move || {
            use std::io::Write;
            let pretty = serde_json::to_string(&args).unwrap_or_default();
            print!("\n⚠  allow `{capability}` with args {pretty}? [y/N] ");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            line.trim().to_lowercase()
        })
        .await
        .unwrap_or_default();

        if answer == "y" || answer == "yes" {
            Decision::Allow
        } else {
            Decision::Deny {
                reason: "denied by user".to_string(),
            }
        }
    }
}
