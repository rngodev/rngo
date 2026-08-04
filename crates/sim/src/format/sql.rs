use serde_json::Value;

use crate::effect::EffectEvent;
use crate::format::Format;
use crate::parse::FormatParser;
use crate::{ParseError, spec};

#[derive(Debug)]
pub struct SqlFormat;

impl SqlFormat {
    pub fn parser() -> SqlFormatParser {
        SqlFormatParser {}
    }
}

impl Format for SqlFormat {
    fn format(&self, event: &EffectEvent) -> Result<String, String> {
        let table = event
            .metadata
            .get("table")
            .and_then(|v| v.as_str())
            .unwrap_or(&event.key);

        Ok(match &event.value {
            Value::Null => {
                format!("INSERT INTO {table} VALUES (null);")
            }
            Value::Bool(b) => {
                format!("INSERT INTO {table} VALUES ({b:?});")
            }
            Value::Number(n) => {
                format!("INSERT INTO {table} VALUES ({n});")
            }
            Value::String(s) => {
                format!("INSERT INTO {table} VALUES ({s});")
            }
            Value::Array(a) => {
                let json_array = serde_json::to_string(a).unwrap();
                format!("INSERT INTO {table} VALUES ('{json_array}');")
            }
            Value::Object(map) => {
                let columns = map
                    .keys()
                    .map(|k| format!("\"{k}\""))
                    .collect::<Vec<_>>()
                    .join(", ");

                let values = map
                    .values()
                    .map(|v| match v {
                        Value::Null => "null".to_string(),
                        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
                        Value::Array(a) => {
                            let json_array = serde_json::to_string(a).unwrap();
                            format!("'{}'", json_array.replace('\'', "''"))
                        }
                        Value::Object(o) => {
                            let json_object = serde_json::to_string(o).unwrap();
                            format!("'{}'", json_object.replace('\'', "''"))
                        }
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                format!("INSERT INTO {table} ({columns}) VALUES ({values});")
            }
        })
    }
}

pub struct SqlFormatParser;

impl FormatParser for SqlFormatParser {
    fn should_parse(&self, format: &spec::Format) -> bool {
        format.ftype.as_deref() == Some("sql")
    }

    fn parse(&self, _format: &spec::Format) -> Result<Box<dyn Format>, Vec<ParseError>> {
        Ok(Box::new(SqlFormat))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn event(value: Value, metadata: Value) -> EffectEvent {
        EffectEvent {
            id: 1,
            key: "user".to_string(),
            offset: 0,
            timestamp: Utc::now().fixed_offset(),
            value,
            metadata,
        }
    }

    #[test]
    fn defaults_table_to_effect_key() {
        let event = event(json!({ "id": 1, "name": "alice" }), Value::Null);
        let sql = SqlFormat.format(&event).unwrap();
        assert!(
            sql.starts_with("INSERT INTO user ("),
            "expected INSERT INTO user, got: {sql}"
        );
        assert!(sql.contains("\"id\""));
        assert!(sql.contains("\"name\""));
    }

    #[test]
    fn table_can_be_overridden_by_metadata() {
        let event = event(json!({ "id": 1 }), json!({ "table": "accounts" }));
        let sql = SqlFormat.format(&event).unwrap();
        assert!(
            sql.starts_with("INSERT INTO accounts ("),
            "expected INSERT INTO accounts, got: {sql}"
        );
    }
}
