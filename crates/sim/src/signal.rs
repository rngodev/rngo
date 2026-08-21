use crate::spec;
use crate::util::cel::json_to_cel;
use cel::{Context, Program};
use indexmap::IndexMap;
use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use serde::Serialize;
use std::path::Path;
use thiserror::Error;

#[derive(Clone, Debug, Serialize)]
pub struct SignalOutcome {
    pub value: serde_json::Value,
    pub passed: bool,
}

#[derive(Debug, Error)]
pub enum SignalError {
    #[error("failed to open log database: {0}")]
    OpenLog(String),
    #[error("signal `{key}`: query failed: {message}")]
    Query { key: String, message: String },
    #[error("signal `{key}`: query returned a blob, which is not a supported result type")]
    UnsupportedValue { key: String },
    #[error("signal `{key}`: expect expression failed to compile: {message}")]
    ExpectCompile { key: String, message: String },
    #[error("signal `{key}`: expect expression failed to evaluate: {message}")]
    ExpectEval { key: String, message: String },
    #[error("signal `{key}`: expect expression must evaluate to a bool, got {value:?}")]
    ExpectNotBool { key: String, value: cel::Value },
}

pub fn evaluate_from_log(
    log_path: &Path,
    signals: &IndexMap<String, spec::Signal>,
) -> Result<IndexMap<String, SignalOutcome>, Vec<SignalError>> {
    let connection =
        Connection::open(log_path).map_err(|e| vec![SignalError::OpenLog(e.to_string())])?;
    evaluate(&connection, signals)
}

fn evaluate(
    connection: &Connection,
    signals: &IndexMap<String, spec::Signal>,
) -> Result<IndexMap<String, SignalOutcome>, Vec<SignalError>> {
    let mut outcomes = IndexMap::new();
    let mut errors = vec![];

    for (key, signal) in signals {
        match evaluate_one(connection, key, signal) {
            Ok(outcome) => {
                outcomes.insert(key.clone(), outcome);
            }
            Err(e) => errors.push(e),
        }
    }

    if errors.is_empty() {
        Ok(outcomes)
    } else {
        Err(errors)
    }
}

fn evaluate_one(
    connection: &Connection,
    key: &str,
    signal: &spec::Signal,
) -> Result<SignalOutcome, SignalError> {
    let spec::Signal::Sql { query, expect } = signal;

    let sql_value = connection
        .query_row(query, [], |row| row.get::<_, SqlValue>(0))
        .map_err(|e| SignalError::Query {
            key: key.to_string(),
            message: e.to_string(),
        })?;

    let value = sql_value_to_json(sql_value).ok_or_else(|| SignalError::UnsupportedValue {
        key: key.to_string(),
    })?;

    let program = Program::compile(expect).map_err(|e| SignalError::ExpectCompile {
        key: key.to_string(),
        message: e.to_string(),
    })?;

    let mut ctx = Context::default();
    ctx.add_variable_from_value("result", json_to_cel(value.clone()));

    let result = program.execute(&ctx).map_err(|e| SignalError::ExpectEval {
        key: key.to_string(),
        message: e.to_string(),
    })?;

    let passed = match result {
        cel::Value::Bool(b) => b,
        other => {
            return Err(SignalError::ExpectNotBool {
                key: key.to_string(),
                value: other,
            });
        }
    };

    Ok(SignalOutcome { value, passed })
}

fn sql_value_to_json(value: SqlValue) -> Option<serde_json::Value> {
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

    fn connection_with_outputs() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE outputs (status INTEGER NOT NULL);
                 INSERT INTO outputs (status) VALUES (200), (200), (500);",
            )
            .unwrap();
        connection
    }

    fn signal(query: &str, expect: &str) -> IndexMap<String, spec::Signal> {
        let mut map = IndexMap::new();
        map.insert(
            "check".to_string(),
            spec::Signal::Sql {
                query: query.to_string(),
                expect: expect.to_string(),
            },
        );
        map
    }

    #[test]
    fn passing_signal() {
        let connection = connection_with_outputs();
        let signals = signal(
            "SELECT COUNT(*) FROM outputs WHERE status = 200",
            "result == 2",
        );

        let outcomes = evaluate(&connection, &signals).unwrap();
        let outcome = &outcomes["check"];
        assert_eq!(outcome.value, serde_json::json!(2));
        assert!(outcome.passed);
    }

    #[test]
    fn failing_signal() {
        let connection = connection_with_outputs();
        let signals = signal(
            "SELECT COUNT(*) FROM outputs WHERE status = 500",
            "result == 0",
        );

        let outcomes = evaluate(&connection, &signals).unwrap();
        let outcome = &outcomes["check"];
        assert_eq!(outcome.value, serde_json::json!(1));
        assert!(!outcome.passed);
    }

    #[test]
    fn range_expression() {
        let connection = connection_with_outputs();
        let signals = signal("SELECT COUNT(*) FROM outputs", "result >= 2 && result <= 5");

        let outcomes = evaluate(&connection, &signals).unwrap();
        assert!(outcomes["check"].passed);
    }

    #[test]
    fn invalid_query_reports_error() {
        let connection = connection_with_outputs();
        let signals = signal("SELECT COUNT(*) FROM missing_table", "result == 0");

        let errors = evaluate(&connection, &signals).unwrap_err();
        assert!(matches!(errors[0], SignalError::Query { .. }));
    }

    #[test]
    fn non_bool_expect_reports_error() {
        let connection = connection_with_outputs();
        let signals = signal("SELECT COUNT(*) FROM outputs", "result");

        let errors = evaluate(&connection, &signals).unwrap_err();
        assert!(matches!(errors[0], SignalError::ExpectNotBool { .. }));
    }
}
