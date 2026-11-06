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
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub seed: Option<u64>,
    pub start: Option<String>,
    pub end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenarios: Option<Value>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub prototypes: IndexMap<String, Prototype>,
    pub effects: IndexMap<String, Effect>,
    #[serde(default)]
    pub channels: IndexMap<String, Channel>,
    #[serde(default)]
    pub generators: IndexMap<String, GeneratorType>,
    #[serde(default)]
    pub signals: IndexMap<String, Signal>,
}

impl Spec {
    /// Returns the spec with scenario `name` patched into every prototype that defines it; errors if no prototype does.
    pub fn with_scenario(&self, name: &str) -> Result<Spec, Vec<ParseError>> {
        let mut spec = self.clone();
        let mut found = false;
        for prototype in spec.prototypes.values_mut() {
            if let Some(scenario) = prototype.scenarios.get(name).cloned() {
                found = true;
                if scenario.start.is_some() {
                    prototype.start = scenario.start;
                }
                if scenario.end.is_some() {
                    prototype.end = scenario.end;
                }
            }
        }
        if found {
            Ok(spec)
        } else {
            Err(vec![
                ParseError::invalid(format!("no prototype defines scenario \"{name}\""))
                    .at(["prototypes"]),
            ])
        }
    }
}

/// Named defaults for the effects that name it, which can each override them; a missing part stays unset.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Prototype {
    pub start: Option<String>,
    pub end: Option<String>,
    pub limit: Option<NonZeroU64>,
    pub channel: Option<String>,
    /// Named overrides of `start` and `end`, applied by `rngo eval --scenario`.
    #[serde(default)]
    pub scenarios: IndexMap<String, Scenario>,
}

/// Overrides of a prototype's `start` and `end`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Scenario {
    pub start: Option<String>,
    pub end: Option<String>,
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
    pub prototype: Option<String>,
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
