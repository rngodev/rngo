use rngo::build::*;
use rngo::generator::{
    Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext, GeneratorParseVisitor,
    GeneratorParser, GeneratorResult,
};
use rngo::{BuildError, Dialect, Input, ParseError, Simulation};
use serde_json::{Value, json};
use std::num::NonZeroU64;

#[derive(Debug)]
struct Counter {
    next: u64,
    step: u64,
}

impl Generator for Counter {
    fn next(&mut self, context: &GeneratorContext) -> GeneratorResult {
        let value = json!({ "count": self.next, "offset": context.trigger.sim_offset });
        self.next += self.step;
        value.into()
    }
}

#[derive(Debug)]
struct CounterBuilder {
    step: u64,
}

impl GeneratorBuilder for CounterBuilder {
    fn build(&self, visitor: GeneratorBuildVisitor) -> Result<Box<dyn Generator>, Vec<BuildError>> {
        if self.step == 0 {
            return Err(vec![visitor.error("step must be positive")]);
        }
        Ok(Box::new(Counter {
            next: 0,
            step: self.step,
        }))
    }
}

fn run(builder: CounterBuilder) -> Result<Vec<Input>, Vec<BuildError>> {
    Ok(Simulation::builder()
        .effect("counter", |e| {
            e.trigger_hertz(1.0)
                .limit(NonZeroU64::new(3).unwrap())
                .generator(object().property("counter", builder))
        })
        .run_log_reader(rngo::SimpleEventRunLog::new())
        .build()?
        .filter_map(|step| step.unwrap().into_input())
        .collect())
}

#[test]
fn a_generator_defined_outside_the_crate_runs_inside_the_dsl() {
    let inputs = run(CounterBuilder { step: 2 }).unwrap();
    let counts: Vec<&Value> = inputs.iter().map(|i| &i.data["counter"]["count"]).collect();

    assert_eq!(counts, [&json!(0), &json!(2), &json!(4)]);
    assert!(inputs[0].data["counter"]["offset"].is_i64());
}

#[test]
fn a_generator_defined_outside_the_crate_reports_build_errors() {
    let errors = run(CounterBuilder { step: 0 }).unwrap_err();

    assert!(
        errors
            .iter()
            .any(|e| e.to_string().contains("step must be positive"))
    );
}

struct CounterParser;

impl GeneratorParser for CounterParser {
    fn key(&self) -> &str {
        "counter"
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<ParseError>> {
        match visitor.spec().fields.get("step").and_then(Value::as_u64) {
            Some(step) => Ok(Box::new(CounterBuilder { step })),
            None => Err(vec![visitor.input_error("step", "step must be a number")]),
        }
    }
}

fn counter_spec(counter: Value) -> Value {
    json!({
        "effects": {
            "counter": {
                "trigger": "hz(1, hour)",
                "limit": 3,
                "generator": {
                    "type": "object",
                    "properties": { "counter": counter }
                }
            }
        }
    })
}

#[test]
fn a_registered_generator_parser_handles_its_type_in_a_spec() {
    let inputs: Vec<Input> = Dialect::primitive()
        .with_generator_parser(CounterParser)
        .parse_simulation_json(counter_spec(json!({ "type": "counter", "step": 5 })))
        .unwrap()
        .run_log_reader(rngo::SimpleEventRunLog::new())
        .build()
        .unwrap()
        .filter_map(|step| step.unwrap().into_input())
        .collect();
    let counts: Vec<&Value> = inputs.iter().map(|i| &i.data["counter"]["count"]).collect();

    assert_eq!(counts, [&json!(0), &json!(5), &json!(10)]);
}

#[test]
fn a_registered_generator_parser_reports_errors_at_its_field() {
    let errors = Dialect::primitive()
        .with_generator_parser(CounterParser)
        .parse_simulation_json(counter_spec(json!({ "type": "counter" })))
        .unwrap_err();

    assert!(errors.iter().any(|e| {
        e.path()
            .ends_with(&["counter".to_string(), "step".to_string()])
            && e.to_string().contains("step must be a number")
    }));
}

#[test]
fn a_generator_type_is_unknown_without_its_parser() {
    assert!(
        Dialect::primitive()
            .parse_simulation_json(counter_spec(json!({ "type": "counter", "step": 5 })))
            .is_err()
    );
}
