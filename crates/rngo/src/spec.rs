use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::num::NonZeroU64;

pub use crate::error::ParseError;

pub fn from_value(value: serde_json::Value) -> Result<Spec, Vec<ParseError>> {
    let mut track = serde_path_to_error::Track::new();
    let deserializer = serde_path_to_error::Deserializer::new(value, &mut track);
    serde_path_to_error::deserialize(deserializer).map_err(|e| {
        vec![ParseError::invalid(e.inner().to_string()).at(e.path().to_string().split('.'))]
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Spec {
    pub seed: Option<u64>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub effects: IndexMap<String, Effect>,
    #[serde(default)]
    pub channels: IndexMap<String, Channel>,
    #[serde(default)]
    pub generators: IndexMap<String, GeneratorType>,
    #[serde(default)]
    pub signals: IndexMap<String, Signal>,
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
    pub channel: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub limit: Option<NonZeroU64>,
    pub trigger: Option<TriggerUnion>,
    pub metadata: Option<Value>,
    pub generator: Generator,
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
pub struct Generator {
    #[serde(rename = "type")]
    pub stype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GeneratorType {
    pub generator: Generator,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    #[serde(rename = "type")]
    pub ctype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Signal {
    #[serde(rename = "type")]
    pub stype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}
