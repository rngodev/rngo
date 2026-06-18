mod common;

use common::{BuildErrorTestExt, SpecErrorTestExt};
use rngo_core::build::*;
use rngo_core::{BuildError, Dialect, EffectKey, Simulation, SpecError};

#[test]
fn builder() {
    let mut simulation_builder = Simulation::builder();

    simulation_builder
        .with_effect("number", |e| {
            e.set_schema(number().min(100).max(18));
        })
        .with_effect("object", |e| {
            e.set_schema(
                object()
                    .property("name", string())
                    .property("age", number().min(100).max(18)),
            );
        })
        .with_effect("no_schema", |_e| {})
        .with_effect("nested", |e| {
            e.set_schema(
                object().property(
                    "score",
                    select()
                        .option(1, number().min(100).max(18))
                        .option(1, constant().value(0)),
                ),
            );
        });

    let errors = simulation_builder.build().unwrap_err();

    let number_error = errors
        .iter()
        .find(|e| matches!(e, BuildError::Schema { effect, .. } if effect == "number"))
        .unwrap();

    assert_eq!(number_error.message(), "min is greater than max");
    assert!(number_error.schema_path().unwrap().is_empty());

    let object_error = errors
        .iter()
        .find(|e| matches!(e, BuildError::Schema { effect, .. } if effect == "object"))
        .unwrap();

    assert_eq!(object_error.message(), "min is greater than max");
    let object_path = object_error.schema_path().unwrap();
    assert_eq!(object_path.len(), 1);
    assert_eq!(object_path[0].kind, "property");
    assert_eq!(object_path[0].key, "age");

    let no_schema_error = errors
        .iter()
        .find(|e| matches!(e, BuildError::Effect { effect, .. } if effect == "no_schema"))
        .unwrap();

    assert_eq!(no_schema_error.message(), "schema was not set");
    assert!(matches!(
        no_schema_error,
        BuildError::Effect {
            key: EffectKey::Schema,
            ..
        }
    ));

    let nested_error = errors
        .iter()
        .find(|e| matches!(e, BuildError::Schema { effect, .. } if effect == "nested"))
        .unwrap();

    assert_eq!(nested_error.message(), "min is greater than max");
    let nested_path = nested_error.schema_path().unwrap();
    assert_eq!(nested_path.len(), 2);
    assert_eq!(nested_path[0].kind, "property");
    assert_eq!(nested_path[0].key, "score");
    assert_eq!(nested_path[1].kind, "option");
    assert_eq!(nested_path[1].key, "0");
}

#[test]
fn spec() {
    let json = r#"{
        "effects": {
            "number": {
                "schema": { "type": "number", "min": "not-a-number" }
            },
            "object": {
                "schema": { "type": "object" }
            },
            "unknown": {
                "schema": { "type": "nonexistent" }
            },
            "nested": {
                "schema": {
                    "type": "object",
                    "properties": {
                        "score": {
                            "type": "select",
                            "options": [
                                { "schema": { "type": "number", "min": "not-a-number" } }
                            ]
                        }
                    }
                }
            }
        }
    }"#;

    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    let errors = Dialect::core().parse_simulation_json(value).unwrap_err();

    let by_effect = |key: &'static str| {
        move |e: &&SpecError| {
            e.path()
                .is_some_and(|p| p.get(1).is_some_and(|k| k == key))
        }
    };

    let number_error = errors.iter().find(by_effect("number")).unwrap();
    assert_eq!(number_error.message(), "min must be a number");
    let number_path = number_error.path().unwrap();
    assert_eq!(
        number_path.as_slice(),
        ["effects", "number", "schema", "min"]
    );

    let object_error = errors.iter().find(by_effect("object")).unwrap();
    assert_eq!(object_error.message(), "not specified");
    let object_path = object_error.path().unwrap();
    assert_eq!(
        object_path.as_slice(),
        ["effects", "object", "schema", "properties"]
    );

    let unknown_error = errors
        .iter()
        .find(|e| e.message() == "no schema parser matched")
        .unwrap();
    assert!(unknown_error.path().is_none());

    let nested_error = errors.iter().find(by_effect("nested")).unwrap();
    assert_eq!(nested_error.message(), "min must be a number");
    let nested_path = nested_error.path().unwrap();
    assert_eq!(
        nested_path.as_slice(),
        [
            "effects",
            "nested",
            "schema",
            "properties",
            "score",
            "options",
            "0",
            "schema",
            "min"
        ]
    );
}
