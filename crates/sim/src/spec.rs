mod parse;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub use parse::Dialect;
pub use parse::{FormatParseContext, FormatParser, SchemaParseVisitor, SchemaParser};

#[derive(Error, Debug, Serialize, Deserialize)]
#[error("failed to parse: `{message}`")]
pub struct SpecError {
    pub path: Option<Vec<String>>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Simulation {
    pub seed: Option<u64>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub effects: IndexMap<String, Effect>,
    #[serde(default)]
    pub systems: IndexMap<String, System>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum Trigger {
    Clock { rate: String },
    Effect { key: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TriggerUnion {
    Shorthand(String),
    Full(Trigger),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Effect {
    pub system: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub trigger: Option<TriggerUnion>,
    pub format: Option<Format>,
    pub schema: Schema,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Format {
    #[serde(rename = "type")]
    pub ftype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Schema {
    #[serde(rename = "type")]
    pub stype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct System {
    pub format: Format,
    pub import: SystemImport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemImport {
    pub before: Option<String>,
    pub command: String,
}
