use super::{Signal, SignalOutcome, evaluate_expect};
use crate::RunLog;
use crate::parse::SignalParser;
use crate::spec::{self, ParseError};
use cel::Program;

#[derive(Debug)]
pub struct SqlSignal {
    key: String,
    query: String,
    expect: Option<Program>,
}

impl SqlSignal {
    pub fn parser() -> SqlSignalParser {
        SqlSignalParser {}
    }
}

impl Signal for SqlSignal {
    fn evaluate(&self, run_log: &dyn RunLog) -> SignalOutcome {
        let value = run_log.get_signal_value(&self.query);
        evaluate_expect(&self.key, self.expect.as_ref(), value)
    }
}

pub struct SqlSignalParser;

impl SignalParser for SqlSignalParser {
    fn key(&self) -> &str {
        "sql"
    }

    fn parse(&self, key: &str, signal: &spec::Signal) -> Result<Box<dyn Signal>, Vec<ParseError>> {
        let query = match signal.fields.get("query") {
            Some(value) => match value.as_str() {
                Some(query) => query.to_string(),
                None => {
                    return Err(vec![ParseError::SchemaError {
                        path: Some(vec!["signals".into(), key.into(), "query".into()]),
                        message: "query must be a string".into(),
                    }]);
                }
            },
            None => {
                return Err(vec![ParseError::SchemaError {
                    path: Some(vec!["signals".into(), key.into(), "query".into()]),
                    message: "query is required".into(),
                }]);
            }
        };

        let expect = match signal.fields.get("expect") {
            Some(value) => {
                let Some(expression) = value.as_str() else {
                    return Err(vec![ParseError::SchemaError {
                        path: Some(vec!["signals".into(), key.into(), "expect".into()]),
                        message: "expect must be a string".into(),
                    }]);
                };

                let program = Program::compile(expression).map_err(|e| {
                    vec![ParseError::SchemaError {
                        path: Some(vec!["signals".into(), key.into(), "expect".into()]),
                        message: format!("expect expression failed to compile: {e}"),
                    }]
                })?;

                Some(program)
            }
            None => None,
        };

        Ok(Box::new(SqlSignal {
            key: key.to_string(),
            query,
            expect,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_signal(fields: serde_json::Value) -> spec::Signal {
        serde_json::from_value(fields).unwrap()
    }

    #[test]
    fn parses_query_and_expect() {
        let signal = spec_signal(serde_json::json!({
            "type": "sql",
            "query": "SELECT 1",
            "expect": "result == 1",
        }));

        let result = SqlSignal::parser().parse("check", &signal);
        assert!(result.is_ok());
    }

    #[test]
    fn missing_query_is_an_error() {
        let signal = spec_signal(serde_json::json!({ "type": "sql" }));
        let result = SqlSignal::parser().parse("check", &signal);
        assert!(result.is_err());
    }

    #[test]
    fn non_string_expect_is_an_error() {
        let signal = spec_signal(serde_json::json!({
            "type": "sql",
            "query": "SELECT 1",
            "expect": 1,
        }));
        let result = SqlSignal::parser().parse("check", &signal);
        assert!(result.is_err());
    }

    #[test]
    fn expect_that_fails_to_compile_is_an_error() {
        let signal = spec_signal(serde_json::json!({
            "type": "sql",
            "query": "SELECT 1",
            "expect": "not ( valid cel",
        }));
        let result = SqlSignal::parser().parse("check", &signal);
        assert!(result.is_err());
    }
}
