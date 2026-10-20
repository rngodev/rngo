mod common;

use common::ParseErrorTestExt;
use rngo::{Dialect, ParseError, Simulation};
use std::fmt;

fn build(json: &str) -> Result<Simulation, String> {
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    let simulation_builder = Dialect::primitive()
        .parse_simulation_json(value)
        .map_err(join_errors)?;
    simulation_builder
        .run_log_reader(rngo::SimpleEventRunLog::new())
        .build()
        .map_err(join_errors)
}

fn join_errors<E: fmt::Display>(errors: Vec<E>) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_errors(json: &str) -> Vec<ParseError> {
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    Dialect::primitive()
        .parse_simulation_json(value)
        .unwrap_err()
}

#[test]
fn resolves_custom_generator_independently_per_effect() {
    let json = r#"{
        "seed": 1,
        "start": "2024-01-01",
        "end": "2024-01-02",
        "generators": {
            "title": {
                "generator": {
                    "type": "select",
                    "options": [
                        { "generator": { "type": "constant", "value": "Mr." } },
                        { "generator": { "type": "constant", "value": "Mrs." } },
                        { "generator": { "type": "constant", "value": "Dr." } }
                    ]
                }
            }
        },
        "effects": {
            "a": {
                "trigger": "hz(1, hour)",
                "generator": {
                    "type": "object",
                    "properties": { "title": { "type": "title" } }
                }
            },
            "b": {
                "trigger": "hz(1, hour)",
                "generator": {
                    "type": "object",
                    "properties": { "title": { "type": "title" } }
                }
            }
        }
    }"#;

    let simulation = build(json).unwrap();
    let events: Vec<_> = simulation.flatten().take(20).collect();

    assert!(events.iter().any(|e| e.effect == "a"));
    assert!(events.iter().any(|e| e.effect == "b"));

    for event in &events {
        let title = event.data["title"].as_str().unwrap();
        assert!(
            ["Mr.", "Mrs.", "Dr."].contains(&title),
            "unexpected title {title:?}"
        );
    }
}

#[test]
fn custom_generator_can_reference_another_custom_generator() {
    let json = r#"{
        "seed": 1,
        "start": "2024-01-01",
        "end": "2024-01-02",
        "generators": {
            "inner": { "generator": { "type": "constant", "value": "x" } },
            "outer": { "generator": { "type": "inner" } }
        },
        "effects": {
            "a": {
                "trigger": "hz(1, hour)",
                "generator": { "type": "outer" }
            }
        }
    }"#;

    let simulation = build(json).unwrap();
    let events: Vec<_> = simulation.flatten().take(1).collect();
    assert_eq!(events[0].data, serde_json::json!("x"));
}

#[test]
fn custom_generator_name_cannot_shadow_primitive_type() {
    let json = r#"{
        "generators": {
            "object": { "generator": { "type": "constant", "value": 1 } }
        },
        "effects": {
            "a": { "generator": { "type": "constant", "value": 1 } }
        }
    }"#;

    let errors = parse_errors(json);

    let error = errors
        .iter()
        .find(|e| e.message().contains("primitive generator type"))
        .unwrap();
    assert_eq!(error.path().unwrap().as_slice(), ["generators", "object"]);
}

#[test]
fn cyclical_custom_generator_reference_errors() {
    let json = r#"{
        "generators": {
            "a": { "generator": { "type": "b" } },
            "b": { "generator": { "type": "a" } }
        },
        "effects": {
            "e": { "generator": { "type": "a" } }
        }
    }"#;

    let errors = parse_errors(json);

    let error = errors
        .iter()
        .find(|e| e.message().contains("cyclical"))
        .unwrap();
    assert!(
        error.message().contains("a -> b -> a"),
        "unexpected message: {}",
        error.message()
    );
}

#[test]
fn malformed_custom_generator_errors() {
    let json = r#"{
        "generators": {
            "age": { "generator": { "type": "number", "minimum": "not-a-number" } }
        },
        "effects": {
            "a": { "generator": { "type": "age" } }
        }
    }"#;

    let errors = parse_errors(json);

    let error = errors
        .iter()
        .find(|e| e.message().contains("minimum must be a number"))
        .unwrap();
    assert_eq!(
        error.path().unwrap().as_slice(),
        ["generators", "age", "generator", "minimum"]
    );
}

#[test]
fn unknown_custom_generator_type_still_errors_like_before() {
    let json = r#"{
        "effects": {
            "a": { "generator": { "type": "nonexistent" } }
        }
    }"#;

    let errors = parse_errors(json);
    assert!(
        errors
            .iter()
            .any(|e| e.message() == "no generator parser matched")
    );
}
