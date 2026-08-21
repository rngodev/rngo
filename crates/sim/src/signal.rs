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
    pub input_id: Option<u64>,
    pub timestamp: DateTime<Utc>,
    pub channel: String,
    pub level: Level,
    pub data: String,
}
