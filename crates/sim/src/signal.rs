use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Io {
    Stdout,
    Stderr,
}

#[derive(Debug, Serialize)]
pub struct Signal {
    pub effect_id: Option<u64>,
    pub timestamp: DateTime<Utc>,
    pub system: String,
    pub io: Io,
    pub data: String,
}
