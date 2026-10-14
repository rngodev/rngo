use super::{Probe, ProbeBuilder, ProbeOutcome};
use crate::RunLogReader;
use crate::audit::probe::{CelExpectation, Signal, SignalEval};
use crate::build::BuildError;
use crate::cel::json_to_cel;
use crate::parse::ProbeParser;
use crate::spec::{self, ParseError};
use cel::{Context, Program};

#[derive(Debug)]
pub struct SqlProbe {
    key: String,
    query: String,
    expectation: Option<super::CelExpectation>,
}

impl SqlProbe {
    pub fn parser() -> SqlProbeParser {
        SqlProbeParser {}
    }

    pub fn builder() -> SqlProbeBuilder {
        SqlProbeBuilder::default()
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

impl Probe for SqlProbe {
    fn evaluate(&self, run_log: &dyn RunLogReader) -> ProbeOutcome {
        let value = run_log.query(&self.query);
        evaluate_expect(&self.key, self.expectation.as_ref(), value)
    }
}

fn evaluate_expect(
    key: &str,
    expectation: Option<&CelExpectation>,
    value: Option<serde_json::Value>,
) -> ProbeOutcome {
    let Some(value) = value else {
        return ProbeOutcome::Error {
            error: format!("probe `{key}`: this run log does not support evaluating probes"),
        };
    };

    let Some(expectation) = expectation else {
        return ProbeOutcome::Signals(vec![Signal { value, eval: None }]);
    };

    let mut ctx = Context::default();
    ctx.add_variable_from_value("result", json_to_cel(value.clone()));

    let result = match expectation.program.execute(&ctx) {
        Ok(r) => r,
        Err(e) => {
            return ProbeOutcome::Error {
                error: format!("probe `{key}`: expect expression failed to evaluate: {e}"),
            };
        }
    };

    let passed = match result {
        cel::Value::Bool(b) => b,
        other => {
            return ProbeOutcome::Error {
                error: format!(
                    "probe `{key}`: expect expression must evaluate to a bool, got {other:?}"
                ),
            };
        }
    };

    ProbeOutcome::Signals(vec![Signal {
        value,
        eval: Some(SignalEval {
            expectation: expectation.source.clone(),
            passed,
        }),
    }])
}

pub struct SqlProbeParser;

impl ProbeParser for SqlProbeParser {
    fn key(&self) -> &str {
        "sql"
    }

    fn parse(&self, key: &str, probe: &spec::Probe) -> Result<Box<dyn Probe>, Vec<ParseError>> {
        let query = match probe.fields.get("query") {
            Some(value) => match value.as_str() {
                Some(query) => query.to_string(),
                None => {
                    return Err(vec![ParseError::SchemaError {
                        path: Some(vec!["probes".into(), key.into(), "query".into()]),
                        message: "query must be a string".into(),
                    }]);
                }
            },
            None => {
                return Err(vec![ParseError::SchemaError {
                    path: Some(vec!["probes".into(), key.into(), "query".into()]),
                    message: "query is required".into(),
                }]);
            }
        };

        let expectation = match probe.fields.get("expect") {
            Some(value) => {
                let Some(source) = value.as_str() else {
                    return Err(vec![ParseError::SchemaError {
                        path: Some(vec!["probes".into(), key.into(), "expect".into()]),
                        message: "expect must be a string".into(),
                    }]);
                };

                let expectation = compile_expectation(source).map_err(|message| {
                    vec![ParseError::SchemaError {
                        path: Some(vec!["probes".into(), key.into(), "expect".into()]),
                        message,
                    }]
                })?;

                Some(expectation)
            }
            None => None,
        };

        Ok(Box::new(SqlProbe {
            key: key.to_string(),
            query,
            expectation,
        }))
    }
}

#[derive(Debug, Default)]
pub struct SqlProbeBuilder {
    query: Option<String>,
    expect: Option<String>,
}

impl SqlProbeBuilder {
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

impl ProbeBuilder for SqlProbeBuilder {
    fn build(self: Box<Self>, key: &str) -> Result<Box<dyn Probe>, Vec<BuildError>> {
        let Some(query) = self.query else {
            return Err(vec![BuildError::Probe {
                probe: key.to_string(),
                message: "query is required".into(),
            }]);
        };

        let expectation = match self.expect {
            Some(source) => Some(compile_expectation(&source).map_err(|message| {
                vec![BuildError::Probe {
                    probe: key.to_string(),
                    message,
                }]
            })?),
            None => None,
        };

        Ok(Box::new(SqlProbe {
            key: key.to_string(),
            query,
            expectation,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_probe(fields: serde_json::Value) -> spec::Probe {
        serde_json::from_value(fields).unwrap()
    }

    fn single_signal(outcome: ProbeOutcome) -> Signal {
        match outcome {
            ProbeOutcome::Signals(mut signals) => {
                assert_eq!(signals.len(), 1);
                signals.remove(0)
            }
            ProbeOutcome::Error { error } => panic!("expected a signal, got error: {error}"),
        }
    }

    fn expectation(expression: &str) -> CelExpectation {
        CelExpectation {
            source: expression.into(),
            program: Program::compile(expression).unwrap(),
        }
    }

    #[test]
    fn parses_query_and_expect() {
        let probe = spec_probe(serde_json::json!({
            "type": "sql",
            "query": "SELECT 1",
            "expect": "result == 1",
        }));

        let result = SqlProbe::parser().parse("check", &probe);
        assert!(result.is_ok());
    }

    #[test]
    fn missing_query_is_an_error() {
        let probe = spec_probe(serde_json::json!({ "type": "sql" }));
        let result = SqlProbe::parser().parse("check", &probe);
        assert!(result.is_err());
    }

    #[test]
    fn builder_builds_probe_with_query_and_expect() {
        let builder = SqlProbe::builder().query("SELECT 1").expect("result == 1");
        let result = Box::new(builder).build("check");
        assert!(result.is_ok());
    }

    #[test]
    fn builder_without_query_is_an_error() {
        let result = Box::new(SqlProbe::builder()).build("check");
        assert!(result.is_err());
    }

    #[test]
    fn builder_rejects_expect_that_fails_to_compile() {
        let builder = SqlProbe::builder()
            .query("SELECT 1")
            .expect("not ( valid cel");
        let result = Box::new(builder).build("check");
        assert!(result.is_err());
    }

    #[test]
    fn non_string_expect_is_an_error() {
        let probe = spec_probe(serde_json::json!({
            "type": "sql",
            "query": "SELECT 1",
            "expect": 1,
        }));
        let result = SqlProbe::parser().parse("check", &probe);
        assert!(result.is_err());
    }

    #[test]
    fn expect_that_fails_to_compile_is_an_error() {
        let probe = spec_probe(serde_json::json!({
            "type": "sql",
            "query": "SELECT 1",
            "expect": "not ( valid cel",
        }));
        let result = SqlProbe::parser().parse("check", &probe);
        assert!(result.is_err());
    }

    #[test]
    fn passing_probe() {
        let expect = expectation("result == 2");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(2)));
        let signal = single_signal(outcome);
        assert_eq!(signal.value, serde_json::json!(2));
        assert!(signal.eval.unwrap().passed);
    }

    #[test]
    fn failing_probe() {
        let expect = expectation("result == 0");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(1)));
        let signal = single_signal(outcome);
        assert_eq!(signal.value, serde_json::json!(1));
        assert!(!signal.eval.unwrap().passed);
    }

    #[test]
    fn missing_expect_has_a_value_but_no_result() {
        let outcome = evaluate_expect("check", None, Some(serde_json::json!(1)));
        let signal = single_signal(outcome);
        assert_eq!(signal.value, serde_json::json!(1));
        assert!(signal.eval.is_none());
    }

    #[test]
    fn range_expression() {
        let expect = expectation("result >= 2 && result <= 5");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(3)));
        assert!(single_signal(outcome).eval.unwrap().passed);
    }

    #[test]
    fn missing_value_is_reported_as_unsupported() {
        let expect = expectation("result == 0");
        let outcome = evaluate_expect("check", Some(&expect), None);
        match outcome {
            ProbeOutcome::Error { error } => {
                assert!(error.contains("does not support evaluating probes"))
            }
            ProbeOutcome::Signals(_) => panic!("expected error"),
        }
    }

    #[test]
    fn non_bool_expect_error_is_captured_in_outcome() {
        let expect = expectation("result");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(3)));
        match outcome {
            ProbeOutcome::Error { error } => assert!(error.contains("must evaluate to a bool")),
            ProbeOutcome::Signals(_) => panic!("expected error"),
        }
    }
}
