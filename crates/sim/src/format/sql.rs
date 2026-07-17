use serde_json::Value;

use crate::ParseError;
use crate::format::Format;
use crate::spec::{FormatParseContext, FormatParser};

#[derive(Debug)]
pub struct SqlFormat {
    table_name: String,
}

impl SqlFormat {
    pub fn parser() -> SqlFormatParser {
        SqlFormatParser {}
    }
}

impl Format for SqlFormat {
    fn format(&self, value: &Value) -> String {
        let table = &self.table_name;
        match value {
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
        }
    }
}

pub struct SqlFormatParser;

impl FormatParser for SqlFormatParser {
    fn should_parse(&self, context: &FormatParseContext) -> bool {
        context.is_format_type("sql")
    }

    fn parse(
        &self,
        context: crate::spec::FormatParseContext,
    ) -> Result<Box<dyn Format>, Vec<ParseError>> {
        let format = context.format();
        let table_name: String = format
            .as_ref()
            .and_then(|f| f.fields.get("table"))
            .map(|v| match v.as_str() {
                Some(table) => Ok(table.to_string()),
                None => Err(vec![ParseError::SchemaError {
                    path: None,
                    message: "table must be a string".into(),
                }]),
            })
            .unwrap_or_else(|| Ok(context.effect_key().to_string()))?;

        Ok(Box::new(SqlFormat { table_name }))
    }
}
