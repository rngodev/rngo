use crate::error::{ParseError, ParseErrorKind};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::num::NonZeroU64;

/// The spec format version this rngo reads; a spec with no `version` is version 1.
pub const SPEC_VERSION: u32 = 1;

/// Where changes between spec format versions are explained.
pub const MIGRATION_URL: &str = "https://github.com/rngodev/rngo/blob/main/MIGRATING.md";

/// Parses a spec from a JSON value, checking its `version` and flagging pre-versioning keys before the strict parse.
pub fn from_value(value: serde_json::Value) -> Result<Spec, Vec<ParseError>> {
    check_version(&value)?;
    let mut track = serde_path_to_error::Track::new();
    let deserializer = serde_path_to_error::Deserializer::new(value, &mut track);
    serde_path_to_error::deserialize(deserializer).map_err(|e| {
        vec![ParseError::invalid(e.inner().to_string()).at(e.path().to_string().split('.'))]
    })
}

fn check_version(value: &Value) -> Result<(), Vec<ParseError>> {
    match value.get("version") {
        Some(version) => match version.as_u64() {
            Some(found) if found == u64::from(SPEC_VERSION) => Ok(()),
            Some(found) if found > 0 => Err(vec![
                ParseError::new(ParseErrorKind::UnsupportedVersion {
                    found,
                    supported: SPEC_VERSION,
                })
                .at(["version"]),
            ]),
            _ => Err(vec![
                ParseError::invalid("version must be a positive integer").at(["version"]),
            ]),
        },
        None => match legacy_key(value) {
            Some(path) => Err(vec![
                ParseError::invalid(format!(
                    "`{}` is from a spec format that predates versioning; see {MIGRATION_URL}",
                    path.last().map_or("", String::as_str)
                ))
                .at(path),
            ]),
            None => Ok(()),
        },
    }
}

fn legacy_key(spec: &Value) -> Option<Vec<String>> {
    for key in ["phases", "schemas"] {
        if spec.get(key).is_some() {
            return Some(vec![key.into()]);
        }
    }
    let section = |name: &str| {
        spec.get(name)
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
    };
    for (name, prototype) in section("prototypes") {
        for key in ["effects", "channels"] {
            if prototype.get(key).is_some() {
                return Some(vec!["prototypes".into(), name.clone(), key.into()]);
            }
        }
    }
    None
}

/// The JSON Schema for the document `name` (`spec`, `effect`, `channel`, `generator` or `signal`), or `None` for any other name.
#[cfg(feature = "schema")]
pub fn json_schema(name: &str) -> Option<Value> {
    let schema = match name {
        "spec" => schemars::schema_for!(Spec),
        "effect" => schemars::schema_for!(Effect),
        "channel" => schemars::schema_for!(Channel),
        "generator" => schemars::schema_for!(GeneratorType),
        "signal" => schemars::schema_for!(Signal),
        _ => return None,
    };
    Some(schema.to_value())
}

/// The names `json_schema` accepts.
#[cfg(feature = "schema")]
pub const SCHEMA_NAMES: [&str; 5] = ["spec", "effect", "channel", "generator", "signal"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Spec {
    /// The spec format version; 1 when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// A name for the project.
    pub key: Option<String>,
    /// Seed for the run's random numbers; 1 when omitted.
    pub seed: Option<u64>,
    /// Start of the default prototype: RFC 3339, `YYYY-MM-DD` or a CEL expression.
    pub start: Option<String>,
    /// End of the default prototype, in the same forms as `start`.
    pub end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenarios: Option<Value>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub prototypes: IndexMap<String, Prototype>,
    /// The kinds of input to generate, by key.
    pub effects: IndexMap<String, Effect>,
    /// Where inputs are delivered, by key.
    #[serde(default)]
    pub channels: IndexMap<String, Channel>,
    /// Named custom generator types that effects can reference.
    #[serde(default)]
    pub generators: IndexMap<String, GeneratorType>,
    /// Checks run once after the simulation, by key.
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Scenario {
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum Trigger {
    Clock { rate: String },
    Effect { key: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
#[non_exhaustive]
pub enum TriggerUnion {
    Shorthand(String),
    Full(Trigger),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Effect {
    /// Key of the prototype whose window and defaults this effect uses.
    pub prototype: Option<String>,
    /// Key of the channel this effect's inputs are delivered to.
    pub channel: Option<String>,
    /// When the effect starts, overriding its prototype's.
    pub start: Option<String>,
    /// When the effect ends, overriding its prototype's.
    pub end: Option<String>,
    /// Maximum number of inputs the effect produces.
    pub limit: Option<NonZeroU64>,
    /// What fires the effect: a clock rate expression or another effect.
    pub trigger: Option<TriggerUnion>,
    /// Extra data attached to every input.
    pub metadata: Option<Value>,
    /// How each input's data is generated.
    pub generator: Generator,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Format {
    #[serde(rename = "type")]
    pub ftype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Generator {
    #[serde(rename = "type")]
    pub stype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct GeneratorType {
    pub generator: Generator,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Channel {
    #[serde(rename = "type")]
    pub ctype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Signal {
    #[serde(rename = "type")]
    pub stype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: Value) -> Result<Spec, Vec<ParseError>> {
        from_value(value)
    }

    #[test]
    fn missing_version_is_accepted() {
        assert!(parse(json!({ "seed": 1, "effects": {} })).is_ok());
    }

    #[test]
    fn current_version_is_accepted() {
        let spec = parse(json!({ "version": 1, "seed": 1, "effects": {} })).unwrap();
        assert_eq!(spec.version, Some(1));
    }

    #[test]
    fn newer_version_points_at_migration_notes() {
        let errors = parse(json!({ "version": 2, "effects": {} })).unwrap_err();
        let message = errors[0].to_string();
        assert!(message.contains("version 2"), "{message}");
        assert!(message.contains(MIGRATION_URL), "{message}");
    }

    #[test]
    fn newer_version_wins_over_unknown_fields() {
        let errors = parse(json!({ "version": 2, "phases": {}, "effects": {} })).unwrap_err();
        assert!(matches!(
            errors[0].kind(),
            ParseErrorKind::UnsupportedVersion { found: 2, .. }
        ));
    }

    #[test]
    fn invalid_version_is_rejected() {
        for version in [json!(0), json!("1"), json!(-1), json!(1.5)] {
            let errors = parse(json!({ "version": version, "effects": {} })).unwrap_err();
            assert!(errors[0].to_string().contains("positive integer"));
        }
    }

    #[test]
    fn legacy_keys_point_at_migration_notes() {
        let specs = [
            json!({ "phases": {}, "effects": {} }),
            json!({ "schemas": {}, "effects": {} }),
            json!({ "effects": {}, "prototypes": { "p": { "effects": [] } } }),
        ];
        for spec in specs {
            let errors = parse(spec).unwrap_err();
            assert!(errors[0].to_string().contains(MIGRATION_URL));
        }
    }

    #[test]
    fn legacy_keys_are_plain_unknown_fields_once_versioned() {
        let errors = parse(json!({ "version": 1, "phases": {}, "effects": {} })).unwrap_err();
        assert!(!errors[0].to_string().contains(MIGRATION_URL));
    }
}
