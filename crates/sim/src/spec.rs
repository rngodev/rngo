mod parse;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub use parse::{
    ConstantSelectOptions, Dialect, FormatParseContext, FormatParser, SchemaParseVisitor,
    SchemaParser,
};

pub fn from_value(value: serde_json::Value) -> Result<Simulation, Vec<SpecError>> {
    let mut track = serde_path_to_error::Track::new();
    let deserializer = serde_path_to_error::Deserializer::new(value, &mut track);
    serde_path_to_error::deserialize(deserializer).map_err(|e| {
        vec![SpecError {
            path: Some(e.path().to_string().split('.').map(String::from).collect()),
            message: e.inner().to_string(),
        }]
    })
}

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
    #[serde(default)]
    pub schemas: IndexMap<String, SchemaType>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SchemaType {
    pub schema: Schema,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct System {
    pub format: Option<Format>,
    pub import: SystemImport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum SystemImport {
    Stream { command: String },
    Exec { command: String },
}
