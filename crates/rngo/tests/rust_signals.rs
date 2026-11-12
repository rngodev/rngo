use rngo::signal::{Signal, SignalEval, SignalOutcome, SignalParser};
use rngo::{Dialect, ParseError, RunLogReader};
use serde_json::{Value, json};

#[derive(Debug)]
struct LastIdAtLeast {
    min: u64,
}

impl Signal for LastIdAtLeast {
    fn evaluate(&self, run_log: &dyn RunLogReader) -> SignalOutcome {
        let Some(last) = run_log.last() else {
            return SignalOutcome::Error {
                error: "no inputs".into(),
            };
        };
        SignalOutcome::Success {
            value: json!(last.id),
            eval: Some(SignalEval {
                expectation: format!("id >= {}", self.min),
                passed: last.id >= self.min,
            }),
        }
    }
}

struct LastIdAtLeastParser;

impl SignalParser for LastIdAtLeastParser {
    fn key(&self) -> &str {
        "last_id_at_least"
    }

    fn parse(
        &self,
        _key: &str,
        signal: &rngo::spec::Signal,
    ) -> Result<Box<dyn Signal>, Vec<ParseError>> {
        match signal.fields.get("min").and_then(Value::as_u64) {
            Some(min) => Ok(Box::new(LastIdAtLeast { min })),
            None => Err(vec![
                ParseError::invalid("min must be a number").at(["min"]),
            ]),
        }
    }
}

fn spec(min: Value) -> rngo::spec::Spec {
    rngo::spec::from_value(json!({
        "effects": {
            "e": {
                "trigger": "hz(1, hour)",
                "limit": 3,
                "generator": { "type": "constant", "value": 1 }
            }
        },
        "signals": {
            "enough": { "type": "last_id_at_least", "min": min }
        }
    }))
    .unwrap()
}

fn outcome(min: u64) -> SignalOutcome {
    let dialect = Dialect::primitive().with_signal_parser(LastIdAtLeastParser);
    let mut simulation = dialect
        .parse_simulation(spec(json!(min)))
        .unwrap()
        .standalone()
        .unwrap();
    for step in &mut simulation {
        step.unwrap();
    }
    let report = dialect
        .parse_audit(spec(json!(min)))
        .unwrap()
        .run_log(simulation.run_log().clone())
        .build()
        .unwrap()
        .run()
        .unwrap();
    report.outcomes["enough"].clone()
}

#[test]
fn a_registered_signal_parser_evaluates_its_signal_in_an_audit() {
    let SignalOutcome::Success {
        eval: Some(passing),
        ..
    } = outcome(1)
    else {
        panic!("expected a success");
    };
    let SignalOutcome::Success {
        eval: Some(failing),
        ..
    } = outcome(100)
    else {
        panic!("expected a success");
    };

    assert!(passing.passed);
    assert!(!failing.passed);
}

#[test]
fn a_registered_signal_parser_reports_parse_errors() {
    let errors = Dialect::primitive()
        .with_signal_parser(LastIdAtLeastParser)
        .parse_audit(spec(json!("lots")))
        .err()
        .unwrap();

    assert!(
        errors
            .iter()
            .any(|e| e.to_string().contains("min must be a number"))
    );
}
