mod clock;
pub mod merge;
pub mod schema;
pub mod source;
mod trigger;

use crate::run_log::Metadata;
use chrono::{DateTime, FixedOffset};
use schema::Metadata as SchemaMetadata;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use trigger::TriggerEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Input {
    pub id: u64,
    pub effect: String,
    pub offset: u64,
    pub timestamp: DateTime<FixedOffset>,
    pub data: Value,
    pub metadata: Vec<SchemaMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedInput {
    pub effect: String,
    pub offset: u64,
    pub timestamp: DateTime<FixedOffset>,
    pub metadata: Vec<SchemaMetadata>,
}

impl From<SkippedInput> for Metadata {
    fn from(skipped: SkippedInput) -> Self {
        let SkippedInput {
            effect,
            offset,
            metadata,
            ..
        } = skipped;

        Metadata {
            mtype: "skipped".to_string(),
            input_id: None,
            output_id: None,
            offset: Some(offset),
            data: Some(serde_json::json!({ "effect": effect, "metadata": metadata })),
            segment: None,
        }
    }
}
