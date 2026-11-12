use rngo::Dialect;
use serde_json::{Value, json};

fn spec(prototypes: Value, effects: Value) -> Value {
    json!({
        "seed": 1,
        "start": "2024-01-01",
        "end": "2024-01-11",
        "channels": { "db": { "type": "stdout" }, "api": { "type": "stdout" } },
        "prototypes": prototypes,
        "effects": effects,
    })
}

fn effect(prototype: Option<&str>, extra: Value) -> Value {
    let mut effect = json!({
        "trigger": "hz(1, day)",
        "generator": { "type": "constant", "value": 1 },
    });
    if let Some(prototype) = prototype {
        effect["prototype"] = json!(prototype);
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
        .filter_map(|step| step.unwrap().into_input())
        .filter(|input| input.effect == effect)
        .map(|input| input.timestamp.format("%Y-%m-%d").to_string())
        .collect()
}

#[test]
fn effects_follow_their_prototype_bounds_and_stay_time_ordered() {
    let value = spec(
        json!({ "late": { "start": "2024-01-06", "end": "2024-01-11" } }),
        json!({ "a": effect(None, json!({})), "b": effect(Some("late"), json!({})) }),
    );
    let simulation = Dialect::primitive()
        .parse_simulation_json(value.clone())
        .unwrap()
        .standalone()
        .unwrap();
    let timestamps: Vec<_> = simulation
        .filter_map(|step| step.unwrap().into_input())
        .map(|i| i.timestamp)
        .collect();
    assert!(timestamps.windows(2).all(|w| w[0] <= w[1]));

    let b = dates(value, "b");
    assert_eq!(b.first().map(String::as_str), Some("2024-01-06"));
    assert_eq!(b.last().map(String::as_str), Some("2024-01-10"));
}

#[test]
fn effect_without_prototype_uses_default_prototype() {
    let value = spec(
        json!({ "late": { "start": "2024-01-06", "end": "2024-01-11" } }),
        json!({ "a": effect(None, json!({})) }),
    );
    assert_eq!(dates(value, "a").first().unwrap(), "2024-01-01");
}

#[test]
fn an_effect_overrides_its_prototype() {
    let value = spec(
        json!({ "late": { "start": "2024-01-06", "end": "2024-01-11" } }),
        json!({ "a": effect(Some("late"), json!({ "start": "2024-01-02", "end": "2024-01-04" })) }),
    );
    let a = dates(value, "a");
    assert_eq!(a.first().unwrap(), "2024-01-02");
    assert_eq!(a.last().unwrap(), "2024-01-03");
}

#[test]
fn an_effect_expression_can_refer_to_its_prototype() {
    let value = spec(
        json!({ "late": { "start": "2024-01-06", "end": "2024-01-11" } }),
        json!({ "a": effect(Some("late"), json!({ "end": "prototype.end - days(2)" })) }),
    );
    let a = dates(value, "a");
    assert_eq!(a.first().unwrap(), "2024-01-06");
    assert_eq!(a.last().unwrap(), "2024-01-08");
}

#[test]
fn an_effect_without_a_prototype_refers_to_the_default_prototype() {
    let value = spec(
        json!({}),
        json!({ "a": effect(None, json!({ "start": "prototype.start + days(3)" })) }),
    );
    assert_eq!(dates(value, "a").first().unwrap(), "2024-01-04");
}

#[test]
fn a_prototype_expression_refers_to_the_default_prototype_as_prototype() {
    let value = spec(
        json!({ "late": { "start": "prototype.start + days(5)", "end": "prototype.end" } }),
        json!({ "a": effect(Some("late"), json!({})) }),
    );
    assert_eq!(dates(value, "a").first().unwrap(), "2024-01-06");
}

#[test]
fn an_effect_can_refer_to_a_prototype_that_only_has_a_start() {
    let value = spec(
        json!({ "p": { "start": "2024-01-06" } }),
        json!({ "a": effect(Some("p"), json!({ "end": "prototype.start + days(2)" })) }),
    );
    let a = dates(value, "a");
    assert_eq!(a.first().unwrap(), "2024-01-06");
    assert_eq!(a.last().unwrap(), "2024-01-07");
}

#[test]
fn referring_to_a_missing_prototype_bound_is_an_error() {
    let value = spec(
        json!({ "p": { "start": "2024-01-06" } }),
        json!({ "a": effect(Some("p"), json!({ "end": "prototype.end" })) }),
    );
    assert!(!errors(value).is_empty());
}

#[test]
fn an_expression_cannot_refer_to_the_simulation() {
    let value = spec(
        json!({ "late": { "start": "simulation.start", "end": "2024-01-11" } }),
        json!({ "a": effect(Some("late"), json!({})) }),
    );
    assert!(!errors(value).is_empty());
}

#[test]
fn an_effect_expression_cannot_refer_to_the_simulation() {
    let value = spec(
        json!({}),
        json!({ "a": effect(None, json!({ "end": "simulation.end" })) }),
    );
    assert!(!errors(value).is_empty());
}

#[test]
fn a_prototype_does_not_inherit_from_the_default_prototype() {
    let value = spec(
        json!({ "late": { "start": "2024-01-06" } }),
        json!({ "a": effect(Some("late"), json!({})) }),
    );
    let errors = errors(value);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("end or limit must be set")),
        "{errors:?}"
    );
}

#[test]
fn a_prototype_limit_caps_its_effects_unless_they_set_their_own() {
    let value = spec(
        json!({ "few": { "start": "2024-01-01", "limit": 3 } }),
        json!({
            "a": effect(Some("few"), json!({})),
            "b": effect(Some("few"), json!({ "limit": 2 })),
        }),
    );
    assert_eq!(dates(value.clone(), "a").len(), 3);
    assert_eq!(dates(value, "b").len(), 2);
}

#[test]
fn a_limit_and_an_end_stop_at_whichever_comes_first() {
    let value = spec(
        json!({}),
        json!({ "a": effect(None, json!({ "limit": 20 })) }),
    );
    let capped = dates(value, "a");
    assert!(capped.len() < 20);
    assert!(capped.iter().all(|date| date.as_str() <= "2024-01-11"));

    let value = spec(
        json!({}),
        json!({ "a": effect(None, json!({ "limit": 4 })) }),
    );
    assert_eq!(dates(value, "a").len(), 4);
}

#[test]
fn invalid_prototypes_are_rejected() {
    let effects = json!({ "a": effect(None, json!({})) });
    let cases = [
        (
            json!({ "p": { "start": "2024-01-09", "end": "2024-01-05" } }),
            "start must be before end",
        ),
        (json!({ "p": { "effects": ["a"] } }), "effects"),
        (json!({ "p": { "channels": ["db"] } }), "channels"),
    ];
    for (prototypes, expected) in cases {
        let errors = errors(spec(prototypes.clone(), effects.clone()));
        assert!(
            errors.iter().any(|e| e.contains(expected)),
            "{prototypes}: {errors:?}"
        );
    }
}

#[test]
fn an_unknown_prototype_names_the_effect_and_the_prototype() {
    let value = spec(json!({}), json!({ "a": effect(Some("nope"), json!({})) }));
    let errors = errors(value);
    assert!(
        errors.iter().any(|e| e.starts_with("effects.a.prototype:")
            && e.contains("\"a\"")
            && e.contains("\"nope\"")),
        "{errors:?}"
    );
}

#[test]
fn phases_are_no_longer_a_spec_field() {
    let mut value = spec(json!({}), json!({ "a": effect(None, json!({})) }));
    value["phases"] = json!({});
    assert!(rngo::spec::from_value(value).is_err());
}

fn proxy_errors(value: Value) -> Vec<String> {
    match Dialect::primitive().parse_proxy(rngo::spec::from_value(value).unwrap()) {
        Err(errors) => errors
            .iter()
            .map(|e| format!("{}: {}", e.path().join("."), e))
            .collect(),
        Ok(_) => vec![],
    }
}

#[test]
fn a_prototype_channel_applies_unless_the_effect_sets_its_own() {
    let mut value = spec(
        json!({ "p": { "start": "2024-01-01", "limit": 1, "channel": "db" } }),
        json!({ "a": effect(Some("p"), json!({})) }),
    );
    assert!(proxy_errors(value.clone()).is_empty());

    value["effects"]["a"]["channel"] = json!("api");
    assert!(proxy_errors(value).is_empty());
}

#[test]
fn an_unknown_prototype_channel_is_reported_once_on_the_prototype() {
    let value = spec(
        json!({ "p": { "start": "2024-01-01", "limit": 1, "channel": "nope" } }),
        json!({ "a": effect(Some("p"), json!({})), "b": effect(Some("p"), json!({})) }),
    );
    let errors = proxy_errors(value);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("prototypes.p.channel:"), "{errors:?}");
}

#[test]
fn top_level_scenarios_are_rejected() {
    let mut value = spec(json!({}), json!({ "a": effect(None, json!({})) }));
    value["scenarios"] = json!({ "x": {} });
    assert!(errors(value).iter().any(|e| e.contains("no scenarios")));
}

#[test]
fn scenario_patches_only_prototypes_defining_it() {
    let value = spec(
        json!({
            "p": { "start": "2024-01-05", "scenarios": { "long": { "start": "2024-01-03" } } },
            "q": { "start": "2024-01-05" },
        }),
        json!({}),
    );
    let parsed = rngo::spec::from_value(value).unwrap();
    let patched = parsed.with_scenario("long").unwrap();
    assert_eq!(patched.prototypes["p"].start.as_deref(), Some("2024-01-03"));
    assert_eq!(patched.prototypes["q"].start.as_deref(), Some("2024-01-05"));
    assert!(parsed.with_scenario("missing").is_err());
}
