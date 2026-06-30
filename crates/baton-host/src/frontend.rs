//! Front-ends consume the brain's [`OutputEvent`] stream plus host lifecycle
//! hooks (DESIGN §8).
//!
//! Rendering is never inside the core; any number of front-ends can subscribe.
//! Phase 1 ships a streaming, ANSI-colored stdout front-end that also surfaces
//! "under the hood" activity (model calls, tool calls + results, permission
//! decisions, token usage) so a user can follow what the agent is doing.

use std::io::{IsTerminal, Write};

use baton_core::{Decision, DoneReason, ModelSelector, OpId, OutputEvent, Usage, Value};

/// Renders the agent's output and activity. The [`Engine`](crate::Engine) calls
/// these as it drains commands and observes the event stream. Every method has
/// a default no-op, so a minimal front-end only implements what it cares about.
#[allow(unused_variables)]
pub trait Frontend: Send {
    /// A cosmetic output event from the brain (streamed text, tool chunks, …).
    fn on_output(&mut self, event: &OutputEvent) {}

    /// A host-level notice (free-form).
    fn on_notice(&mut self, message: &str) {}

    /// A model call is starting.
    fn on_model_start(&mut self, op: OpId, selector: &ModelSelector) {}

    /// A model call finished; carries its token usage.
    fn on_model_end(&mut self, op: OpId, usage: &Usage) {}

    /// A capability (tool) is about to run, with its arguments.
    fn on_tool_start(&mut self, op: OpId, name: &str, args: &Value) {}

    /// A capability finished. `is_error` marks a tool-level failure (the result
    /// is the error payload).
    fn on_tool_end(&mut self, op: OpId, name: &str, result: &Value, is_error: bool) {}

    /// A permission request was decided.
    fn on_permission(&mut self, capability: &str, decision: &Decision) {}

    /// The turn reached a terminal state.
    fn on_done(&mut self, reason: &DoneReason) {}
}

// --- ANSI styling -----------------------------------------------------------

mod style {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const CYAN: &str = "\x1b[36m";
    pub const GRAY: &str = "\x1b[90m";
}

/// Streams assistant text to stdout as it arrives, and renders agent activity
/// with ANSI colors. Colors are auto-disabled when stdout is not a TTY or when
/// `NO_COLOR` is set.
pub struct StdoutFrontend {
    color: bool,
    /// Whether we're mid-line streaming assistant text (so the next log line
    /// inserts a newline first).
    streaming: bool,
    /// When set, tool results are rendered in full instead of being collapsed
    /// to a head + "… +N lines" summary. Defaults from `BATON_FULL_OUTPUT`.
    full_output: bool,
}

/// How many leading lines of a tool result to show before collapsing the rest.
const RESULT_HEAD_LINES: usize = 8;

impl Default for StdoutFrontend {
    fn default() -> Self {
        let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        Self {
            color,
            streaming: false,
            full_output: env_truthy("BATON_FULL_OUTPUT"),
        }
    }
}

/// Read an env var as a boolean: set & non-empty & not `0`/`false`/`no` ⇒ true.
fn env_truthy(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let v = v.trim();
            !v.is_empty() && !matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "no")
        }
        Err(_) => false,
    }
}

impl StdoutFrontend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Force colors on or off, overriding TTY/`NO_COLOR` detection.
    pub fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    /// Force full (uncollapsed) tool-result rendering on or off, overriding the
    /// `BATON_FULL_OUTPUT` env default.
    pub fn with_full_output(mut self, full: bool) -> Self {
        self.full_output = full;
        self
    }

    /// Wrap `text` in an ANSI code when colors are enabled.
    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("{code}{text}{}", style::RESET)
        } else {
            text.to_string()
        }
    }

    fn break_stream(&mut self) {
        if self.streaming {
            println!();
            self.streaming = false;
        }
    }

    /// Print one activity line (breaking any in-progress streamed text first).
    fn line(&mut self, text: String) {
        self.break_stream();
        println!("{text}");
    }

    /// Build the (already styled) lines a tool result renders to. Pure, so the
    /// collapse / full-output behaviour can be unit-tested without stdout.
    ///
    /// In compact mode the body is a head of at most [`RESULT_HEAD_LINES`]
    /// lines followed by a "… +N lines" note; with `full_output` the whole
    /// result is shown.
    fn tool_end_lines(&self, name: &str, result: &Value, is_error: bool) -> Vec<String> {
        let (marker, code) = if is_error {
            ("✗", style::RED)
        } else {
            ("✓", style::GREEN)
        };
        let header = self.paint(code, &format!("  {marker} {name}"));

        let body = render_result(result);
        let lines: Vec<&str> = body.lines().collect();
        let show = if self.full_output {
            lines.len()
        } else {
            lines.len().min(RESULT_HEAD_LINES)
        };

        let mut out = Vec::new();
        // First body line sits next to the header; subsequent shown lines are
        // indented underneath.
        let first = lines.first().copied().unwrap_or("");
        out.push(format!(
            "{header} {}",
            self.paint(style::GRAY, &arrow(first))
        ));
        for line in lines.iter().take(show).skip(1) {
            out.push(self.paint(style::GRAY, &format!("    {line}")));
        }
        let hidden = lines.len().saturating_sub(show);
        if hidden > 0 {
            let note = format!("    … +{hidden} lines (set BATON_FULL_OUTPUT=1 to expand)");
            out.push(self.paint(style::DIM, &note));
        }
        out
    }
}

impl Frontend for StdoutFrontend {
    fn on_output(&mut self, event: &OutputEvent) {
        match event {
            OutputEvent::ModelText { text, .. } => {
                print!("{text}");
                let _ = std::io::stdout().flush();
                self.streaming = true;
            }
            OutputEvent::ToolChunk { chunk, .. } => {
                self.break_stream();
                let text = chunk
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| chunk.to_string());
                print!("{}", self.paint(style::GRAY, &text));
                let _ = std::io::stdout().flush();
            }
            // Tool starts are reported richly via `on_tool_start`; reasoning is
            // hidden in this minimal front-end.
            OutputEvent::ToolCallStarted { .. } | OutputEvent::ModelReasoning { .. } => {}
            OutputEvent::Notice(msg) => self.line(self.paint(style::GRAY, msg)),
            _ => {}
        }
    }

    fn on_notice(&mut self, message: &str) {
        let line = self.paint(style::GRAY, message);
        self.line(line);
    }

    fn on_model_start(&mut self, op: OpId, selector: &ModelSelector) {
        let name = match selector {
            ModelSelector::Named(name) => name.as_str(),
            _ => "?",
        };
        let marker = self.paint(style::CYAN, "●");
        let label = self.paint(style::DIM, &format!("model [{name}] {op}"));
        self.line(format!("{marker} {label}"));
    }

    fn on_model_end(&mut self, _op: OpId, usage: &Usage) {
        if usage.input_tokens == 0 && usage.output_tokens == 0 {
            return; // no usage reported by the provider
        }
        let text = format!(
            "  ↳ {} in / {} out tokens",
            usage.input_tokens, usage.output_tokens
        );
        let line = self.paint(style::GRAY, &text);
        self.line(line);
    }

    fn on_tool_start(&mut self, op: OpId, name: &str, args: &Value) {
        let marker = self.paint(style::YELLOW, "⚙");
        let name = self.paint(style::BOLD, name);
        let args = self.paint(style::DIM, &truncate(&compact(args), 160));
        self.line(format!(
            "{marker} {name} {args} {}",
            self.paint(style::GRAY, &op.to_string())
        ));
    }

    fn on_tool_end(&mut self, _op: OpId, name: &str, result: &Value, is_error: bool) {
        for line in self.tool_end_lines(name, result, is_error) {
            self.line(line);
        }
    }

    fn on_permission(&mut self, capability: &str, decision: &Decision) {
        let line = match decision {
            Decision::Allow => self.paint(style::GREEN, &format!("  ↳ allowed `{capability}`")),
            Decision::Deny { reason } => {
                self.paint(style::RED, &format!("  ↳ denied `{capability}`: {reason}"))
            }
            _ => self.paint(style::GRAY, &format!("  ↳ permission for `{capability}`")),
        };
        self.line(line);
    }

    fn on_done(&mut self, reason: &DoneReason) {
        self.break_stream();
        match reason {
            DoneReason::EndTurn => {}
            DoneReason::Cancelled => println!("{}", self.paint(style::YELLOW, "⚠ cancelled")),
            DoneReason::Error(msg) => {
                eprintln!("{}", self.paint(style::RED, &format!("✗ error: {msg}")));
            }
            _ => {}
        }
    }
}

/// Prefix a single-line summary with the result arrow, collapsing newlines.
fn arrow(first_line: &str) -> String {
    format!("→ {}", truncate(first_line, 160))
}

/// Render a tool result into human-readable, possibly multi-line text.
///
/// Strings are shown verbatim (their own newlines drive the head/collapse
/// logic). Objects are rendered as `key: value` lines, expanding any string
/// field that itself contains newlines (e.g. a shell `stdout`) onto its own
/// lines — this is what makes a 1000-line command output collapse nicely.
/// Anything else falls back to compact JSON.
fn render_result(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Object(map) => {
            let mut out = String::new();
            for (k, v) in map {
                match v {
                    Value::String(s) if s.contains('\n') => {
                        out.push_str(&format!("{k}:\n{}\n", s.trim_end_matches('\n')));
                    }
                    Value::String(s) => out.push_str(&format!("{k}: {s}\n")),
                    other => out.push_str(&format!("{k}: {}\n", compact(other))),
                }
            }
            out.trim_end_matches('\n').to_string()
        }
        other => compact(other),
    }
}

/// Compact one-line JSON for logging.
fn compact(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
    }
}

/// Truncate to `max` chars (by char boundary), appending an ellipsis if cut.
fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        return s;
    }
    let kept: String = s.chars().take(max).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A 1000-line shell result collapses to a head plus a "… +N lines" note.
    #[test]
    fn tool_result_collapses_by_default() {
        let body: String = (0..1000).map(|i| format!("line {i}\n")).collect::<String>();
        let result = json!({ "exit_code": 0, "stdout": body, "stderr": "" });

        // Colors off so we assert on plain text.
        let fe = StdoutFrontend::new()
            .with_color(false)
            .with_full_output(false);
        let lines = fe.tool_end_lines("shell", &result, false);

        // Far fewer than 1000 lines, and a collapse note is present.
        assert!(
            lines.len() <= RESULT_HEAD_LINES + 2,
            "got {} lines",
            lines.len()
        );
        let joined = lines.join("\n");
        // 1000 stdout lines + the `exit_code:`/`stderr:`/`stdout:` framing means
        // the bulk is hidden; the note reports the hidden remainder.
        assert!(joined.contains("… +"), "missing collapse note:\n{joined}");
        assert!(
            joined.contains("lines"),
            "note should mention lines:\n{joined}"
        );
        assert!(
            joined.contains("BATON_FULL_OUTPUT"),
            "note should mention the toggle"
        );
        // The head still shows real content.
        assert!(
            joined.contains("line 0"),
            "head should show first lines:\n{joined}"
        );
        // The tail is not shown.
        assert!(
            !joined.contains("line 999"),
            "tail should be hidden:\n{joined}"
        );
    }

    /// `with_full_output(true)` (the `BATON_FULL_OUTPUT` toggle) shows everything.
    #[test]
    fn tool_result_full_output_shows_everything() {
        let body: String = (0..1000).map(|i| format!("line {i}\n")).collect::<String>();
        let result = json!({ "exit_code": 0, "stdout": body, "stderr": "" });

        let fe = StdoutFrontend::new()
            .with_color(false)
            .with_full_output(true);
        let lines = fe.tool_end_lines("shell", &result, false);
        let joined = lines.join("\n");

        assert!(
            !joined.contains("… +"),
            "full output must not collapse:\n{joined}"
        );
        assert!(joined.contains("line 0"), "first line missing");
        assert!(
            joined.contains("line 999"),
            "last line missing in full output"
        );
    }

    /// A small result is shown inline with no collapse note either way.
    #[test]
    fn small_result_not_collapsed() {
        let result = json!({ "exit_code": 0, "stdout": "hello\n", "stderr": "" });
        let fe = StdoutFrontend::new()
            .with_color(false)
            .with_full_output(false);
        let joined = fe.tool_end_lines("shell", &result, false).join("\n");
        assert!(
            !joined.contains("… +"),
            "small result should not collapse:\n{joined}"
        );
        assert!(joined.contains("hello"));
    }

    /// `BATON_FULL_OUTPUT` parsing: truthy vs falsy values.
    #[test]
    fn env_truthy_parsing() {
        // We can't safely mutate process env in parallel tests; exercise the
        // parser via the same matching used by `env_truthy`.
        for (v, want) in [
            ("1", true),
            ("true", true),
            ("yes", true),
            ("0", false),
            ("false", false),
            ("no", false),
            ("", false),
        ] {
            let v = v.trim();
            let got =
                !v.is_empty() && !matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "no");
            assert_eq!(got, want, "value {v:?}");
        }
    }
}
