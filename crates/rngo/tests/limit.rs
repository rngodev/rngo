mod common;

use common::ParseErrorTestExt;
use rngo::build::*;
use rngo::{Dialect, Input, SimpleEventRunLog, Simulation};
use std::num::NonZeroU64;

fn count_for(inputs: &[Input], effect: &str) -> usize {
    inputs.iter().filter(|i| i.effect == effect).count()
}

fn limit(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}

#[test]
fn unlimited_effect_keeps_running_after_limited_effect_stops() {
    let mut simulation_builder = Simulation::builder();
    simulation_builder
        .with_effect("limited", |e| {
            e.trigger_hertz(1.0)
                .limit(limit(3))
                .schema(constant().value(1))
        })
        .with_effect("unlimited", |e| {
            e.trigger_expression("hz(1, hour)".into())
                .schema(constant().value(1))
        });

    let inputs: Vec<_> = simulation_builder.build().unwrap().collect();

    assert_eq!(count_for(&inputs, "limited"), 3);
    assert!(count_for(&inputs, "unlimited") > 3);

    let last_limited = inputs.iter().rfind(|i| i.effect == "limited").unwrap();
    assert!(
        inputs
            .iter()
            .any(|i| i.effect == "unlimited" && i.offset > last_limited.offset),
        "the unlimited effect should keep producing after the limit is reached"
    );
}

#[test]
fn dependent_effect_stops_after_upstream_limit() {
    let run_log = SimpleEventRunLog::new();

    let mut simulation_builder = Simulation::builder();
    simulation_builder
        .with_effect("upstream", |e| {
            e.trigger_expression("hz(1, hour)".into())
                .limit(limit(4))
                .schema(constant().value(1))
        })
        .with_effect("downstream", |e| {
            e.trigger_effect("upstream".into())
                .schema(constant().value(2))
        });

    let inputs: Vec<_> = simulation_builder
        .run_log(run_log)
        .build()
        .unwrap()
        .collect();

    assert_eq!(count_for(&inputs, "upstream"), 4);
    assert_eq!(count_for(&inputs, "downstream"), 4);
}

#[test]
fn run_limit_lower_than_effect_limit_wins() {
    let mut simulation_builder = Simulation::builder();
    simulation_builder.with_effect("ping", |e| {
        e.trigger_hertz(1.0)
            .limit(limit(10))
            .schema(constant().value(1))
    });

    let inputs: Vec<_> = simulation_builder.limit(4).build().unwrap().collect();

    assert_eq!(inputs.len(), 4);
}

#[test]
fn effect_limit_lower_than_run_limit_wins() {
    let mut simulation_builder = Simulation::builder();
    simulation_builder.with_effect("ping", |e| {
        e.trigger_hertz(1.0)
            .limit(limit(4))
            .schema(constant().value(1))
    });

    let inputs: Vec<_> = simulation_builder.limit(10).build().unwrap().collect();

    assert_eq!(inputs.len(), 4);
}

#[test]
fn spec_limit_is_applied() {
    let value = serde_json::json!({
        "seed": 1,
        "start": "2024-01-01",
        "end": "2024-01-02",
        "effects": {
            "ping": {
                "trigger": "hz(1, hour)",
                "limit": 5,
                "schema": { "type": "constant", "value": 1 }
            }
        }
    });

    let inputs: Vec<_> = Dialect::primitive()
        .parse_simulation_json(value)
        .unwrap()
        .build()
        .unwrap()
        .collect();

    assert_eq!(inputs.len(), 5);
}

#[test]
fn invalid_spec_limit_is_rejected_with_effect_path() {
    for invalid in [
        serde_json::json!(0),
        serde_json::json!(-1),
        serde_json::json!(1.5),
    ] {
        let value = serde_json::json!({
            "seed": 1,
            "start": "2024-01-01",
            "end": "2024-01-02",
            "effects": {
                "ping": {
                    "trigger": "hz(1, hour)",
                    "limit": invalid,
                    "schema": { "type": "constant", "value": 1 }
                }
            }
        });

        let errors = Dialect::primitive()
            .parse_simulation_json(value)
            .unwrap_err();

        assert_eq!(errors.len(), 1, "limit {invalid} should be rejected");
        assert_eq!(
            errors[0].path().unwrap().as_slice(),
            ["effects", "ping", "limit"],
            "limit {invalid} should be rejected at the effect's limit path"
        );
        assert!(
            errors[0].to_string().contains("effects.ping.limit"),
            "error for limit {invalid} should name the effect: {}",
            errors[0]
        );
    }
}
