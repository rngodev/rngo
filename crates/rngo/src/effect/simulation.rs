use crate::RunLogReader;
use crate::build::{BuildError, SimulationKey};
use crate::effect::{Effect, EffectBuilder, Input, SkippedInput};
use crate::moment::Moment;
use chrono::{TimeDelta, Utc};
use std::rc::Rc;

#[derive(Debug)]
pub struct Simulation {
    effects: Vec<Effect>,
    limit: Option<u64>,
    emitted: u64,
}

impl Simulation {
    pub fn builder() -> SimulationBuilder {
        SimulationBuilder::new()
    }
}

impl Iterator for Simulation {
    type Item = Result<Input, SkippedInput>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.limit.is_some_and(|limit| self.emitted >= limit) {
            return None;
        }

        self.effects
            .sort_unstable_by_key(|e| e.next_offset().unwrap_or(i64::MAX));

        let item = self.effects.first_mut()?.next()?;
        self.emitted += 1;
        Some(item)
    }
}

#[derive(Debug)]
pub struct SimulationBuilder {
    pub seed: u64,
    pub start: Moment,
    pub end: Moment,
    run_log_reader: Option<Rc<dyn RunLogReader>>,
    effect_builders: Vec<EffectBuilder>,
    limit: Option<u64>,
}

impl SimulationBuilder {
    fn new() -> Self {
        SimulationBuilder {
            seed: 1,
            start: Moment::Relative(TimeDelta::days(-30)),
            end: Moment::Relative(TimeDelta::zero()),
            run_log_reader: None,
            effect_builders: vec![],
            limit: None,
        }
    }

    pub fn run_log_reader<T: RunLogReader + 'static>(mut self, reader: Rc<T>) -> Self {
        self.run_log_reader = Some(reader as Rc<dyn RunLogReader>);
        self
    }

    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.set_seed(seed);
        self
    }

    pub fn set_seed(&mut self, seed: u64) -> &mut Self {
        self.seed = seed;
        self
    }

    pub fn start(mut self, start: Moment) -> Self {
        self.set_start(start);
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.start = start;
        self
    }

    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.end = end;
        self
    }

    pub fn set_effect(&mut self, effect: EffectBuilder) {
        self.effect_builders.push(effect)
    }

    pub fn with_effect(
        &mut self,
        key: &str,
        f: impl FnOnce(EffectBuilder) -> EffectBuilder,
    ) -> &mut Self {
        let builder = Effect::builder(key.into());
        let builder = f(builder);
        self.effect_builders.push(builder);
        self
    }

    pub fn build(self) -> Result<Simulation, Vec<BuildError>> {
        let mut errors = vec![];
        let now = Utc::now().fixed_offset();
        let start = self.start.resolve(now);
        let end = self.end.resolve(now);

        if start >= end {
            errors.push(BuildError::Simulation {
                key: SimulationKey::Start,
                message: "start must be before end".into(),
            });
        }

        let Some(run_log_reader) = self.run_log_reader else {
            errors.push(BuildError::Simulation {
                key: SimulationKey::RunLogReader,
                message: "run_log_reader was not set".into(),
            });
            return Err(errors);
        };

        let mut effects = vec![];

        for mut effect_builder in self.effect_builders {
            effect_builder
                .set_now(now)
                .set_sim_start(start)
                .set_sim_end(end)
                .set_event_run_log(run_log_reader.clone())
                .set_seed(self.seed);

            match effect_builder.build() {
                Ok(effect) => effects.push(effect),
                Err(mut e) => errors.append(&mut e),
            }
        }

        if errors.is_empty() {
            Ok(Simulation {
                effects,
                limit: self.limit,
                emitted: 0,
            })
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::build::BuildError;
    use crate::effect::schema::{
        Metadata, Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult,
    };

    #[derive(Debug, Default)]
    struct AlternatingSchema {
        calls: u32,
    }

    impl Schema for AlternatingSchema {
        fn next(&mut self, _context: &SchemaContext) -> SchemaResult {
            self.calls += 1;
            if self.calls % 2 == 1 {
                SchemaResult {
                    value: Some(serde_json::Value::Null),
                    metadata: vec![],
                }
            } else {
                SchemaResult {
                    value: None,
                    metadata: vec![Metadata {
                        mtype: "error".into(),
                        attribute: None,
                        data: Some(serde_json::json!({ "message": "boom" })),
                    }],
                }
            }
        }
    }

    #[derive(Debug)]
    struct AlternatingSchemaBuilder;

    impl SchemaBuilder for AlternatingSchemaBuilder {
        fn build(&self, _visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
            Ok(Box::new(AlternatingSchema::default()))
        }
    }

    #[test]
    fn limit_counts_effects_and_errors_together() {
        let mut simulation_builder =
            super::Simulation::builder().run_log_reader(crate::SimpleEventRunLog::new());

        simulation_builder.with_effect("alternating", |e| {
            e.trigger_hertz(1000.0).schema(AlternatingSchemaBuilder)
        });

        let items: Vec<_> = simulation_builder.limit(5).build().unwrap().collect();

        assert_eq!(
            items.len(),
            5,
            "limit should count both real and skipped attempts toward the cap"
        );
        assert_eq!(items.iter().filter(|item| item.is_ok()).count(), 3);
    }

    #[test]
    fn build_without_a_run_log_reader_is_an_error() {
        let errors = super::Simulation::builder().build().unwrap_err();

        assert!(matches!(
            errors.as_slice(),
            [BuildError::Simulation {
                key: crate::SimulationKey::RunLogReader,
                ..
            }]
        ));
    }
}
