use crate::metadata::Metadata;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Input {
    pub id: u64,
    pub effect: String,
    pub offset: u64,
    pub timestamp: DateTime<FixedOffset>,
    pub data: Value,
    pub metadata: Vec<Metadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedInput {
    pub effect: String,
    pub offset: u64,
    pub timestamp: DateTime<FixedOffset>,
    pub metadata: Vec<Metadata>,
}
