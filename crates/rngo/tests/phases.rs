use rngo::Dialect;
use serde_json::{Value, json};

fn spec(phases: Value, effects: Value) -> Value {
    json!({
        "seed": 1,
        "start": "2024-01-01",
        "end": "2024-01-11",
        "channels": { "db": { "type": "stdout" }, "api": { "type": "stdout" } },
        "phases": phases,
        "effects": effects,
    })
}

fn effect(channel: Option<&str>, extra: Value) -> Value {
    let mut effect = json!({
        "trigger": "hz(1, day)",
        "generator": { "type": "constant", "value": 1 },
    });
    if let Some(channel) = channel {
        effect["channel"] = json!(channel);
    }
    effect
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    effect
}

fn errors(value: Value) -> Vec<String> {
    match Dialect::primitive().parse_simulation_json(value) {
        Err(errors) => errors
            .iter()
            .map(|e| format!("{}: {}", e.path().join("."), e))
            .collect(),
        Ok(builder) => match builder.standalone() {
            Err(errors) => errors.iter().map(|e| e.to_string()).collect(),
            Ok(_) => vec![],
        },
    }
}

fn dates(value: Value, effect: &str) -> Vec<String> {
    let simulation = Dialect::primitive()
        .parse_simulation_json(value)
        .unwrap()
        .standalone()
        .unwrap();
    simulation
        .flatten()
        .filter(|input| input.effect == effect)
        .map(|input| input.timestamp.format("%Y-%m-%d").to_string())
        .collect()
}

#[test]
fn effects_follow_their_phase_bounds_and_stay_time_ordered() {
    let value = spec(
        json!({ "late": { "start": "2024-01-06", "channels": ["api"] } }),
        json!({ "a": effect(Some("db"), json!({})), "b": effect(Some("api"), json!({})) }),
    );
    let simulation = Dialect::primitive()
        .parse_simulation_json(value.clone())
        .unwrap()
        .standalone()
        .unwrap();
    let timestamps: Vec<_> = simulation.flatten().map(|i| i.timestamp).collect();
    assert!(timestamps.windows(2).all(|w| w[0] <= w[1]));

    let b = dates(value, "b");
    assert_eq!(b.first().map(String::as_str), Some("2024-01-06"));
    assert_eq!(b.last().map(String::as_str), Some("2024-01-10"));
}

#[test]
fn effects_list_beats_channels() {
    let value = spec(
        json!({ "late": { "start": "2024-01-06", "channels": ["db"], "effects": ["b"] } }),
        json!({ "a": effect(Some("db"), json!({})), "b": effect(Some("api"), json!({})) }),
    );
    assert_eq!(dates(value.clone(), "a").first().unwrap(), "2024-01-06");
    assert_eq!(dates(value, "b").first().unwrap(), "2024-01-06");
}

#[test]
fn effect_without_channel_uses_default_phase() {
    let value = spec(
        json!({ "late": { "start": "2024-01-06", "channels": ["db"] } }),
        json!({ "a": effect(None, json!({})) }),
    );
    assert_eq!(dates(value, "a").first().unwrap(), "2024-01-01");
}

#[test]
fn effect_outside_its_phase_is_rejected() {
    let value = spec(
        json!({ "late": { "start": "2024-01-06", "channels": ["db"] } }),
        json!({ "a": effect(Some("db"), json!({ "start": "2024-01-02" })) }),
    );
    let errors = errors(value);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("before phase \"late\" start")),
        "{errors:?}"
    );
}

#[test]
fn invalid_phases_are_rejected() {
    let effects = json!({ "a": effect(Some("db"), json!({})), "b": effect(None, json!({})) });
    let cases = [
        (
            json!({ "p": { "start": "2024-01-09", "end": "2024-01-05" } }),
            "start must be before end",
        ),
        (
            json!({ "p": { "channels": ["db"] }, "q": { "channels": ["db"] } }),
            "already claimed",
        ),
        (
            json!({ "p": { "effects": ["a"] }, "q": { "effects": ["a"] } }),
            "already in phase",
        ),
        (json!({ "p": { "effects": ["nope"] } }), "unknown effect"),
        (json!({ "p": { "channels": ["nope"] } }), "nope"),
    ];
    for (phases, expected) in cases {
        let errors = errors(spec(phases.clone(), effects.clone()));
        assert!(
            errors.iter().any(|e| e.contains(expected)),
            "{phases}: {errors:?}"
        );
    }
}

#[test]
fn top_level_scenarios_are_rejected() {
    let mut value = spec(json!({}), json!({ "a": effect(None, json!({})) }));
    value["scenarios"] = json!({ "x": {} });
    assert!(errors(value).iter().any(|e| e.contains("no scenarios")));
}

#[test]
fn scenario_patches_only_phases_defining_it() {
    let value = spec(
        json!({
            "p": { "channels": ["db"], "start": "2024-01-05", "scenarios": { "long": { "start": "2024-01-03" } } },
            "q": { "channels": ["api"], "start": "2024-01-05" },
        }),
        json!({}),
    );
    let parsed = rngo::spec::from_value(value).unwrap();
    let patched = parsed.with_scenario("long").unwrap();
    assert_eq!(patched.phases["p"].start.as_deref(), Some("2024-01-03"));
    assert_eq!(patched.phases["q"].start.as_deref(), Some("2024-01-05"));
    assert!(parsed.with_scenario("missing").is_err());
}
