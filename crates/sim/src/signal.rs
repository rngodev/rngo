use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, Serialize)]
pub struct Signal {
    pub effect_id: Option<u64>,
    /// Set to the signal's spec key when it comes from a non-interactive `.rngo/signals/*.yml`
    /// source rather than an effect.
    pub key: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub system: String,
    pub level: Level,
    pub data: String,
}
