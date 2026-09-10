pub(crate) mod sql;

pub use sql::SqlSignal;

use crate::RunLog;
use crate::util::cel::json_to_cel;
use cel::{Context, Program};
use rusqlite::types::Value as SqlValue;
use serde::Serialize;
use thiserror::Error;

/// A built, runtime-evaluable signal - the result of a [`crate::parse::SignalParser`] parsing a
/// [`crate::spec::Signal`]. Mirrors [`crate::format::Format`]: parsing is a single step directly
/// to this runtime trait, since (unlike schemas) evaluating a signal needs no persistent
/// build-time resource beyond the [`RunLog`] it's handed on each call.
pub trait Signal: std::fmt::Debug {
    fn evaluate(&self, run_log: &dyn RunLog) -> SignalOutcome;
}

/// `value`/`passed` are `None` together only when `error` is `Some` - a signal with no `expect`
/// still has a `value`, just no `passed` verdict.
#[derive(Clone, Debug, Serialize)]
pub struct SignalOutcome {
    pub value: Option<serde_json::Value>,
    pub passed: Option<bool>,
    pub error: Option<String>,
}

impl SignalOutcome {
    pub(crate) fn error(error: SignalError) -> Self {
        SignalOutcome {
            value: None,
            passed: None,
            error: Some(error.to_string()),
        }
    }
}

#[derive(Debug, Error)]
pub enum SignalError {
    #[error("signal `{key}`: this run log does not support evaluating signals")]
    Unsupported { key: String },
    #[error("signal `{key}`: expect expression failed to evaluate: {message}")]
    ExpectEval { key: String, message: String },
    #[error("signal `{key}`: expect expression must evaluate to a bool, got {value:?}")]
    ExpectNotBool { key: String, value: cel::Value },
}

/// Runs a signal's already-compiled `expect` program against `value` - the raw result of
/// fetching the signal's value (e.g. [`sql::SqlSignal`] running its query against a [`RunLog`]),
/// or `None` if the fetch couldn't produce one. Backend-agnostic: every [`Signal`] impl's
/// `evaluate` funnels through here once it has a raw value. Compiling `expect` happens once, at
/// parse time (see [`sql::SqlSignalParser`]), so a bad expression is rejected before a signal
/// ever runs rather than on every evaluation.
pub(crate) fn evaluate_expect(
    key: &str,
    expect: Option<&Program>,
    value: Option<serde_json::Value>,
) -> SignalOutcome {
    let Some(value) = value else {
        return SignalOutcome::error(SignalError::Unsupported {
            key: key.to_string(),
        });
    };

    let Some(program) = expect else {
        return SignalOutcome {
            value: Some(value),
            passed: None,
            error: None,
        };
    };

    let mut ctx = Context::default();
    ctx.add_variable_from_value("result", json_to_cel(value.clone()));

    let result = match program.execute(&ctx) {
        Ok(r) => r,
        Err(e) => {
            return SignalOutcome::error(SignalError::ExpectEval {
                key: key.to_string(),
                message: e.to_string(),
            });
        }
    };

    let passed = match result {
        cel::Value::Bool(b) => b,
        other => {
            return SignalOutcome::error(SignalError::ExpectNotBool {
                key: key.to_string(),
                value: other,
            });
        }
    };

    SignalOutcome {
        value: Some(value),
        passed: Some(passed),
        error: None,
    }
}

pub(crate) fn sql_value_to_json(value: SqlValue) -> Option<serde_json::Value> {
    Some(match value {
        SqlValue::Null => serde_json::Value::Null,
        SqlValue::Integer(i) => serde_json::Value::from(i),
        SqlValue::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        SqlValue::Text(s) => serde_json::Value::String(s),
        SqlValue::Blob(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(expression: &str) -> Program {
        Program::compile(expression).unwrap()
    }

    #[test]
    fn passing_signal() {
        let expect = program("result == 2");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(2)));
        assert_eq!(outcome.value, Some(serde_json::json!(2)));
        assert_eq!(outcome.passed, Some(true));
        assert!(outcome.error.is_none());
    }

    #[test]
    fn failing_signal() {
        let expect = program("result == 0");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(1)));
        assert_eq!(outcome.value, Some(serde_json::json!(1)));
        assert_eq!(outcome.passed, Some(false));
        assert!(outcome.error.is_none());
    }

    #[test]
    fn missing_expect_has_a_value_but_no_result() {
        let outcome = evaluate_expect("check", None, Some(serde_json::json!(1)));
        assert_eq!(outcome.value, Some(serde_json::json!(1)));
        assert_eq!(outcome.passed, None);
        assert!(outcome.error.is_none());
    }

    #[test]
    fn range_expression() {
        let expect = program("result >= 2 && result <= 5");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(3)));
        assert_eq!(outcome.passed, Some(true));
    }

    #[test]
    fn missing_value_is_reported_as_unsupported() {
        let expect = program("result == 0");
        let outcome = evaluate_expect("check", Some(&expect), None);
        assert!(outcome.value.is_none());
        assert!(outcome.passed.is_none());
        assert!(
            outcome
                .error
                .as_ref()
                .unwrap()
                .contains("does not support evaluating signals")
        );
    }

    #[test]
    fn non_bool_expect_error_is_captured_in_outcome() {
        let expect = program("result");
        let outcome = evaluate_expect("check", Some(&expect), Some(serde_json::json!(3)));
        assert!(outcome.value.is_none());
        assert!(outcome.passed.is_none());
        assert!(
            outcome
                .error
                .as_ref()
                .unwrap()
                .contains("must evaluate to a bool")
        );
    }
}
