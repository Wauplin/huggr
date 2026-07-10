//! Rust response contract and extension point for the `hugr-weather` Hugr agent.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const RESPONSE_RUST_TYPE: &str = "hugr_weather::Response";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub response: String,
}
