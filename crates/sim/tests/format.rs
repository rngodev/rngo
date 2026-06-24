mod common;

use common::SpecErrorTestExt;
use rngo_sim::{Dialect, Event, SpecError};

fn parse_and_run(json: &str) -> Vec<Event> {
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    let simulation = Dialect::core()
        .parse_simulation_json(value)
        .unwrap()
        .build()
        .unwrap();
    simulation.take(3).collect()
}

fn parse_errors(json: &str) -> Vec<SpecError> {
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    Dialect::core().parse_simulation_json(value).unwrap_err()
}

fn effect_format(event: &Event) -> Option<&str> {
    match event {
        Event::Effect { format, .. } => format.as_deref(),
        _ => None,
    }
}

#[test]
fn effect_only() {
    let json = r#"{
        "effects": {
            "user": {
                "format": { "type": "sql" },
                "schema": { "type": "constant", "value": { "id": 1, "name": "alice" } }
            }
        }
    }"#;

    let events = parse_and_run(json);
    assert!(!events.is_empty());

    for event in &events {
        let sql = effect_format(event).expect("format should be set");
        assert!(
            sql.starts_with("INSERT INTO user ("),
            "expected INSERT INTO user, got: {sql}"
        );
        assert!(sql.contains("\"id\""), "expected id column, got: {sql}");
        assert!(sql.contains("\"name\""), "expected name column, got: {sql}");
    }
}

#[test]
fn effect_only_custom_table() {
    let json = r#"{
        "effects": {
            "user": {
                "format": { "type": "sql", "table": "accounts" },
                "schema": { "type": "constant", "value": { "id": 1 } }
            }
        }
    }"#;

    let events = parse_and_run(json);
    assert!(!events.is_empty());

    for event in &events {
        let sql = effect_format(event).expect("format should be set");
        assert!(
            sql.starts_with("INSERT INTO accounts ("),
            "expected INSERT INTO accounts, got: {sql}"
        );
    }
}

#[test]
fn inherited_from_system() {
    let json = r#"{
        "systems": {
            "db": {
                "format": { "type": "sql" },
                "import": { "type": "stream", "command": "psql" }
            }
        },
        "effects": {
            "user": {
                "system": "db",
                "schema": { "type": "constant", "value": { "id": 1 } }
            }
        }
    }"#;

    let events = parse_and_run(json);
    assert!(!events.is_empty());

    for event in &events {
        let sql = effect_format(event).expect("format should be inherited from system");
        assert!(
            sql.starts_with("INSERT INTO user ("),
            "expected INSERT INTO user, got: {sql}"
        );
    }
}

#[test]
fn inherited_from_system_effect_overrides_table() {
    let json = r#"{
        "systems": {
            "db": {
                "format": { "type": "sql", "table": "system_default" },
                "import": { "type": "stream", "command": "psql" }
            }
        },
        "effects": {
            "user": {
                "system": "db",
                "format": { "table": "accounts" },
                "schema": { "type": "constant", "value": { "id": 1 } }
            }
        }
    }"#;

    let events = parse_and_run(json);
    assert!(!events.is_empty());

    for event in &events {
        let sql = effect_format(event).expect("format should be present");
        assert!(
            sql.starts_with("INSERT INTO accounts ("),
            "expected INSERT INTO accounts, got: {sql}"
        );
    }
}

#[test]
fn errors() {
    let type_mismatch = r#"{
        "systems": {
            "db": {
                "format": { "type": "sql" },
                "import": { "type": "stream", "command": "psql" }
            }
        },
        "effects": {
            "user": {
                "system": "db",
                "format": { "type": "other" },
                "schema": { "type": "constant", "value": 1 }
            }
        }
    }"#;

    let errors = parse_errors(type_mismatch);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message(),
        "effect format type \"other\" does not match system format type \"sql\""
    );
    assert_eq!(
        errors[0].path(),
        Some(&vec![
            "effects".into(),
            "user".into(),
            "format".into(),
            "type".into()
        ])
    );

    let invalid_table = r#"{
        "effects": {
            "user": {
                "format": { "type": "sql", "table": 42 },
                "schema": { "type": "constant", "value": 1 }
            }
        }
    }"#;

    let errors = parse_errors(invalid_table);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message(), "table must be a string");
}
