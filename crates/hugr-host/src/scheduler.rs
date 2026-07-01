//! Host-side scheduling (ARCHITECTURE §15.2).
//!
//! The core has no clock and no scheduler. A native host fires a schedule by
//! choosing a trace target, optionally resuming it, injecting a `UserInput`, and
//! checkpointing the grown trace.

use std::path::PathBuf;
use std::time::Duration;

use crate::{CheckpointCadence, EngineBuilder, Trace, TraceError};

/// A small cron-like cadence for host schedules.
///
/// Supported forms:
///
/// - `@every 10s`, `@every 5m`, `@every 1h`
/// - `* * * * *` (every minute)
/// - `*/N * * * *` (every N minutes)
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CronExpr {
    source: String,
    interval: Duration,
}

impl CronExpr {
    /// Parse a supported cron cadence.
    pub fn parse(input: impl AsRef<str>) -> Result<Self, ScheduleError> {
        let source = input.as_ref().trim().to_string();
        if let Some(rest) = source.strip_prefix("@every ") {
            return Ok(Self {
                interval: parse_duration(rest.trim())?,
                source,
            });
        }

        let fields: Vec<&str> = source.split_whitespace().collect();
        if fields.len() == 5 && fields[1..].iter().all(|field| *field == "*") {
            let minutes = match fields[0] {
                "*" => 1,
                minute if minute.starts_with("*/") => minute[2..]
                    .parse::<u64>()
                    .map_err(|_| ScheduleError::InvalidCron(source.clone()))?
                    .max(1),
                _ => return Err(ScheduleError::InvalidCron(source)),
            };
            return Ok(Self {
                source,
                interval: Duration::from_secs(minutes * 60),
            });
        }

        Err(ScheduleError::InvalidCron(source))
    }

    /// The interval represented by this cadence. The scheduler sleeps this long
    /// between fires; the core only ever sees injected `Tick` events.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Original user-facing expression.
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Where a scheduled fire writes/continues its trace.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TriggerTarget {
    /// Resume this exact trace if it exists; otherwise return an IO error.
    ResumeExisting { trace: PathBuf },
    /// Resolve `name` under `dir`. If the trace exists it is resumed; otherwise
    /// a new persistent session starts at that path.
    NamedPersistent { dir: PathBuf, name: String },
    /// Start a fresh session and write it to this path.
    FreshSession { trace: PathBuf },
}

impl TriggerTarget {
    /// The trace path this target maps to for the current fire.
    pub fn trace_path(&self) -> PathBuf {
        match self {
            TriggerTarget::ResumeExisting { trace } | TriggerTarget::FreshSession { trace } => {
                trace.clone()
            }
            TriggerTarget::NamedPersistent { dir, name } => dir.join(format!("{name}.trace.json")),
        }
    }
}

/// A host schedule: cadence + target + prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Schedule {
    pub cron: CronExpr,
    pub target: TriggerTarget,
    pub prompt: String,
}

impl Schedule {
    pub fn new(cron: CronExpr, target: TriggerTarget, prompt: impl Into<String>) -> Self {
        Self {
            cron,
            target,
            prompt: prompt.into(),
        }
    }
}

/// Fire one scheduled trigger by running one user turn and checkpointing the
/// resulting trace. The caller supplies a fully configured [`EngineBuilder`]
/// (models, tools, permission policy, front-end); this function only decides
/// whether to resume a trace or start fresh.
pub async fn fire_once(
    builder: EngineBuilder,
    schedule: &Schedule,
) -> Result<PathBuf, ScheduleError> {
    let path = schedule.target.trace_path();
    let builder = match &schedule.target {
        TriggerTarget::ResumeExisting { .. } => {
            let trace = Trace::load(&path)?;
            builder.resume(trace)
        }
        TriggerTarget::NamedPersistent { .. } if path.exists() => {
            let trace = Trace::load(&path)?;
            builder.resume(trace)
        }
        TriggerTarget::NamedPersistent { .. } | TriggerTarget::FreshSession { .. } => builder,
    }
    .checkpoint(path.clone(), CheckpointCadence::EveryEvent);

    let mut engine = builder.build();
    engine.user_turn(schedule.prompt.clone()).await;
    engine.session_end();
    engine.save_trace(&path)?;
    Ok(path)
}

/// Scheduler errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ScheduleError {
    #[error("invalid cron expression: {0}")]
    InvalidCron(String),
    #[error("invalid duration: {0}")]
    InvalidDuration(String),
    #[error(transparent)]
    Trace(#[from] TraceError),
}

fn parse_duration(input: &str) -> Result<Duration, ScheduleError> {
    let (number, multiplier) = match input.chars().last() {
        Some('s') => (&input[..input.len() - 1], 1),
        Some('m') => (&input[..input.len() - 1], 60),
        Some('h') => (&input[..input.len() - 1], 60 * 60),
        Some(c) if c.is_ascii_digit() => (input, 1),
        _ => return Err(ScheduleError::InvalidDuration(input.to_string())),
    };
    let n = number
        .parse::<u64>()
        .map_err(|_| ScheduleError::InvalidDuration(input.to_string()))?;
    Ok(Duration::from_secs(n.max(1) * multiplier))
}
