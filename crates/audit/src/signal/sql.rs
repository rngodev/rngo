use super::{Signal, SignalBuilder, SignalOutcome};
use crate::parse::SignalParser;
use crate::signal::{CelExpectation, SignalEval};
use cel::{Context, Program};
use rngo_core::BuildError;
use rngo_core::spec::{self, ParseError};
use rngo_core::util::cel::json_to_cel;
use rngo_log::RunLogReader;

#[derive(Debug)]
pub struct SqlSignal {
    key: String,
    query: String,
    expectation: Option<super::CelExpectation>,
}

impl SqlSignal {
    pub fn parser() -> SqlSignalParser {
        SqlSignalParser {}
    }

    pub fn builder() -> SqlSignalBuilder {
        SqlSignalBuilder::default()
    }
}

fn compile_expectation(source: &str) -> Result<CelExpectation, String> {
    let program = Program::compile(source)
        .map_err(|e| format!("expect expression failed to compile: {e}"))?;

    Ok(CelExpectation {
        source: source.to_string(),
        program,
    })
}

impl Signal for SqlSignal {
    fn evaluate(&self, run_log: &dyn RunLogReader) -> SignalOutcome {
        let value = run_log.query(&self.query);
        evaluate_expect(&self.key, self.expectation.as_ref(), value)
    }
}

fn evaluate_expect(
    key: &str,
    expectation: Option<&CelExpectation>,
    value: Option<serde_json::Value>,
) -> SignalOutcome {
    let Some(value) = value else {
        return SignalOutcome::Error {
            error: format!("signal `{key}`: this run log does not support evaluating signals"),
        };
    };

    let Some(expectation) = expectation else {
        return SignalOutcome::Success { value, eval: None };
    };

    let mut ctx = Context::default();
    ctx.add_variable_from_value("result", json_to_cel(value.clone()));

    let result = match expectation.program.execute(&ctx) {
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
        eval: Some(SignalEval {
            expectation: expectation.source.clone(),
            passed,
        }),
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

        let expectation = match signal.fields.get("expect") {
            Some(value) => {
                let Some(source) = value.as_str() else {
                    return Err(vec![ParseError::SchemaError {
                        path: Some(vec!["signals".into(), key.into(), "expect".into()]),
                        message: "expect must be a string".into(),
                    }]);
                };

                let expectation = compile_expectation(source).map_err(|message| {
                    vec![ParseError::SchemaError {
                        path: Some(vec!["signals".into(), key.into(), "expect".into()]),
                        message,
                    }]
                })?;

                Some(expectation)
            }
            None => None,
        };

        Ok(Box::new(SqlSignal {
            key: key.to_string(),
            query,
            expectation,
        }))
    }
}

#[derive(Debug, Default)]
pub struct SqlSignalBuilder {
    query: Option<String>,
    expect: Option<String>,
}

impl SqlSignalBuilder {
    pub fn query(mut self, query: impl Into<String>) -> Self {
        self.set_query(query);
        self
    }

    pub fn set_query(&mut self, query: impl Into<String>) -> &mut Self {
        self.query = Some(query.into());
        self
    }

    pub fn expect(mut self, expect: impl Into<String>) -> Self {
        self.set_expect(expect);
        self
    }

    pub fn set_expect(&mut self, expect: impl Into<String>) -> &mut Self {
        self.expect = Some(expect.into());
        self
    }
}

impl SignalBuilder for SqlSignalBuilder {
    fn build(self: Box<Self>, key: &str) -> Result<Box<dyn Signal>, Vec<BuildError>> {
        let Some(query) = self.query else {
            return Err(vec![BuildError::Signal {
                signal: key.to_string(),
                message: "query is required".into(),
            }]);
        };

        let expectation = match self.expect {
            Some(source) => Some(compile_expectation(&source).map_err(|message| {
                vec![BuildError::Signal {
                    signal: key.to_string(),
                    message,
                }]
            })?),
            None => None,
        };

        Ok(Box::new(SqlSignal {
            key: key.to_string(),
            query,
            expectation,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_signal(fields: serde_json::Value) -> spec::Signal {
        serde_json::from_value(fields).unwrap()
    }

    fn expectation(expression: &str) -> CelExpectation {
        CelExpectation {
            source: expression.into(),
            program: Program::compile(expression).unwrap(),
        }
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
    fn builder_builds_signal_with_query_and_expect() {
        let builder = SqlSignal::builder().query("SELECT 1").expect("result == 1");
        let result = Box::new(builder).build("check");
        assert!(result.is_ok());
    }

    #[test]
    fn builder_without_query_is_an_error() {
        let result = Box::new(SqlSignal::builder()).build("check");
        assert!(result.is_err());
    }

    #[test]
    fn builder_rejects_expect_that_fails_to_compile() {
        let builder = SqlSignal::builder()
            .query("SELECT 1")
            .expect("not ( valid cel");
        let result = Box::new(builder).build("check");
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
        let expect = expectation("result == 2");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(2)));
        match outcome {
            SignalOutcome::Success { value, eval } => {
                assert_eq!(value, serde_json::json!(2));
                assert!(eval.unwrap().passed);
            }
            SignalOutcome::Error { error } => panic!("expected success, got error: {error}"),
        }
    }

    #[test]
    fn failing_signal() {
        let expect = expectation("result == 0");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(1)));
        match outcome {
            SignalOutcome::Success { value, eval } => {
                assert_eq!(value, serde_json::json!(1));
                assert!(!eval.unwrap().passed);
            }
            SignalOutcome::Error { error } => panic!("expected success, got error: {error}"),
        }
    }

    #[test]
    fn missing_expect_has_a_value_but_no_result() {
        let outcome = evaluate_expect("check", None, Some(serde_json::json!(1)));
        match outcome {
            SignalOutcome::Success { value, eval } => {
                assert_eq!(value, serde_json::json!(1));
                assert!(eval.is_none());
            }
            SignalOutcome::Error { error } => panic!("expected success, got error: {error}"),
        }
    }

    #[test]
    fn range_expression() {
        let expect = expectation("result >= 2 && result <= 5");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(3)));
        match outcome {
            SignalOutcome::Success { eval, .. } => assert!(eval.unwrap().passed),
            SignalOutcome::Error { error } => panic!("expected success, got error: {error}"),
        }
    }

    #[test]
    fn missing_value_is_reported_as_unsupported() {
        let expect = expectation("result == 0");
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
        let expect = expectation("result");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(3)));
        match outcome {
            SignalOutcome::Error { error } => assert!(error.contains("must evaluate to a bool")),
            SignalOutcome::Success { .. } => panic!("expected error"),
        }
    }
}
