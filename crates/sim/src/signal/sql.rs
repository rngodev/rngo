use super::{Signal, SignalOutcome};
use crate::RunLogReader;
use crate::parse::SignalParser;
use crate::spec::{self, ParseError};
use crate::util::cel::json_to_cel;
use cel::{Context, Program};

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
    fn evaluate(&self, run_log: &dyn RunLogReader) -> SignalOutcome {
        let value = run_log.query(&self.query);
        evaluate_expect(&self.key, self.expect.as_ref(), value)
    }
}

/// Runs an already-compiled `expect` program against `value` - the raw result of running the
/// signal's query against a [`RunLogReader`], or `None` if the run log couldn't produce one. Compiling
/// `expect` happens once, at parse time (see [`SqlSignalParser::parse`]), so a bad expression is
/// rejected before a signal ever runs rather than on every evaluation.
fn evaluate_expect(
    key: &str,
    expect: Option<&Program>,
    value: Option<serde_json::Value>,
) -> SignalOutcome {
    let Some(value) = value else {
        return SignalOutcome::Error {
            error: format!("signal `{key}`: this run log does not support evaluating signals"),
        };
    };

    let Some(program) = expect else {
        return SignalOutcome::Success {
            value,
            passed: None,
        };
    };

    let mut ctx = Context::default();
    ctx.add_variable_from_value("result", json_to_cel(value.clone()));

    let result = match program.execute(&ctx) {
        Ok(r) => r,
        Err(e) => {
            return SignalOutcome::Error {
                error: format!("signal `{key}`: expect expression failed to evaluate: {e}"),
            };
        }
    };

    let passed = match result {
        cel::Value::Bool(b) => b,
        other => {
            return SignalOutcome::Error {
                error: format!(
                    "signal `{key}`: expect expression must evaluate to a bool, got {other:?}"
                ),
            };
        }
    };

    SignalOutcome::Success {
        value,
        passed: Some(passed),
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

    fn program(expression: &str) -> Program {
        Program::compile(expression).unwrap()
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

    #[test]
    fn passing_signal() {
        let expect = program("result == 2");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(2)));
        match outcome {
            SignalOutcome::Success { value, passed } => {
                assert_eq!(value, serde_json::json!(2));
                assert_eq!(passed, Some(true));
            }
            SignalOutcome::Error { error } => panic!("expected success, got error: {error}"),
        }
    }

    #[test]
    fn failing_signal() {
        let expect = program("result == 0");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(1)));
        match outcome {
            SignalOutcome::Success { value, passed } => {
                assert_eq!(value, serde_json::json!(1));
                assert_eq!(passed, Some(false));
            }
            SignalOutcome::Error { error } => panic!("expected success, got error: {error}"),
        }
    }

    #[test]
    fn missing_expect_has_a_value_but_no_result() {
        let outcome = evaluate_expect("check", None, Some(serde_json::json!(1)));
        match outcome {
            SignalOutcome::Success { value, passed } => {
                assert_eq!(value, serde_json::json!(1));
                assert_eq!(passed, None);
            }
            SignalOutcome::Error { error } => panic!("expected success, got error: {error}"),
        }
    }

    #[test]
    fn range_expression() {
        let expect = program("result >= 2 && result <= 5");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(3)));
        match outcome {
            SignalOutcome::Success { passed, .. } => assert_eq!(passed, Some(true)),
            SignalOutcome::Error { error } => panic!("expected success, got error: {error}"),
        }
    }

    #[test]
    fn missing_value_is_reported_as_unsupported() {
        let expect = program("result == 0");
        let outcome = evaluate_expect("check", Some(&expect), None);
        match outcome {
            SignalOutcome::Error { error } => {
                assert!(error.contains("does not support evaluating signals"))
            }
            SignalOutcome::Success { .. } => panic!("expected error"),
        }
    }

    #[test]
    fn non_bool_expect_error_is_captured_in_outcome() {
        let expect = program("result");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(3)));
        match outcome {
            SignalOutcome::Error { error } => assert!(error.contains("must evaluate to a bool")),
            SignalOutcome::Success { .. } => panic!("expected error"),
        }
    }
}
