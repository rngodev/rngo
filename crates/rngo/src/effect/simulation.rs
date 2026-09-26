use crate::build::{BuildError, SimulationKey};
use crate::effect::{Effect, EffectBuilder, Input};
use crate::log::{Metadata, SimpleEventRunLog};
use crate::moment::Moment;
use crate::{RunLogReader, RunLogWriter};
use chrono::{TimeDelta, Utc};
use std::rc::Rc;

#[derive(Debug)]
pub struct Simulation {
    effects: Vec<Effect>,
    writer: Rc<dyn RunLogWriter>,
    limit: Option<u64>,
    emitted: u64,
    started: bool,
    finished: bool,
}

impl Simulation {
    pub fn builder() -> SimulationBuilder {
        SimulationBuilder::new()
    }

    pub fn finish(&mut self) {
        if self.started && !self.finished {
            self.finished = true;
            self.record_wall_clock("simulation_end");
        }
    }

    fn record_wall_clock(&self, mtype: &str) {
        self.writer.push_metadata(Metadata {
            mtype: mtype.to_string(),
            input_id: None,
            output_id: None,
            offset: None,
            data: Some(Utc::now().to_rfc3339().into()),
            segment: None,
        });
    }

    fn advance(&mut self) -> Option<Input> {
        loop {
            if self.limit.is_some_and(|limit| self.emitted >= limit) {
                return None;
            }

            self.effects
                .sort_unstable_by_key(|e| e.next_offset().unwrap_or(u64::MAX));

            match self.effects.first_mut()?.next()? {
                Ok(input) => {
                    self.emitted += 1;
                    self.writer.push_input(input.clone());
                    return Some(input);
                }
                Err(skipped_input) => {
                    self.emitted += 1;
                    self.writer.push_metadata(skipped_input.into());
                }
            }
        }
    }
}

impl Iterator for Simulation {
    type Item = Input;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        if !self.started {
            self.started = true;
            self.record_wall_clock("simulation_start");
        }

        let input = self.advance();
        if input.is_none() {
            self.finish();
        }
        input
    }
}

impl Drop for Simulation {
    fn drop(&mut self) {
        self.finish();
    }
}

#[derive(Debug)]
pub struct SimulationBuilder {
    pub seed: u64,
    pub start: Moment,
    pub end: Moment,
    run_log_reader: Option<Rc<dyn RunLogReader>>,
    run_log_writer: Option<Rc<dyn RunLogWriter>>,
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
            run_log_writer: None,
            effect_builders: vec![],
            limit: None,
        }
    }

    pub fn run_log_reader<T: RunLogReader + 'static>(mut self, reader: Rc<T>) -> Self {
        self.run_log_reader = Some(reader as Rc<dyn RunLogReader>);
        self
    }

    pub fn run_log_writer<T: RunLogWriter + 'static>(mut self, writer: Rc<T>) -> Self {
        self.run_log_writer = Some(writer as Rc<dyn RunLogWriter>);
        self
    }

    pub fn run_log<T: RunLogReader + RunLogWriter + 'static>(self, run_log: Rc<T>) -> Self {
        self.run_log_reader(run_log.clone()).run_log_writer(run_log)
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

        let (run_log_reader, run_log_writer) = match (self.run_log_reader, self.run_log_writer) {
            (Some(reader), Some(writer)) => (reader, writer),
            _ => {
                let default_run_log = SimpleEventRunLog::new();
                (
                    default_run_log.clone() as Rc<dyn RunLogReader>,
                    default_run_log as Rc<dyn RunLogWriter>,
                )
            }
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
                writer: run_log_writer,
                limit: self.limit,
                emitted: 0,
                started: false,
                finished: false,
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
        let mut simulation_builder = super::Simulation::builder();

        simulation_builder.with_effect("alternating", |e| {
            e.trigger_hertz(1000.0).schema(AlternatingSchemaBuilder)
        });

        let inputs: Vec<_> = simulation_builder.limit(5).build().unwrap().collect();

        assert_eq!(
            inputs.len(),
            3,
            "limit should count both real and skipped attempts toward the cap"
        );
    }

    #[derive(Debug, Default)]
    struct RecordedMetadata(std::cell::RefCell<Vec<crate::Metadata>>);

    impl crate::RunLogWriter for RecordedMetadata {
        fn push_input(&self, _input: crate::Input) {}
        fn push_output(&self, _output: crate::Output) {}
        fn push_metadata(&self, metadata: crate::Metadata) {
            self.0.borrow_mut().push(metadata);
        }
    }

    fn wall_clock_metadata(writer: &RecordedMetadata) -> Vec<crate::Metadata> {
        writer
            .0
            .borrow()
            .iter()
            .filter(|metadata| metadata.mtype.starts_with("simulation_"))
            .cloned()
            .collect()
    }

    fn wall_clock_types(writer: &RecordedMetadata) -> Vec<String> {
        wall_clock_metadata(writer)
            .into_iter()
            .map(|metadata| metadata.mtype)
            .collect()
    }

    fn wall_clock_times(writer: &RecordedMetadata) -> Vec<chrono::DateTime<chrono::FixedOffset>> {
        wall_clock_metadata(writer)
            .iter()
            .map(|metadata| {
                let data = metadata
                    .data
                    .as_ref()
                    .and_then(|data| data.as_str())
                    .unwrap();
                chrono::DateTime::parse_from_rfc3339(data).unwrap()
            })
            .collect()
    }

    fn simulation_with(writer: std::rc::Rc<RecordedMetadata>) -> super::Simulation {
        let mut simulation_builder = super::Simulation::builder()
            .run_log_reader(crate::SimpleEventRunLog::new())
            .run_log_writer(writer);

        simulation_builder.with_effect("alternating", |e| {
            e.trigger_hertz(1000.0).schema(AlternatingSchemaBuilder)
        });

        simulation_builder.limit(5).build().unwrap()
    }

    #[test]
    fn records_wall_clock_start_and_end_when_exhausted() {
        let writer = std::rc::Rc::new(RecordedMetadata::default());
        let mut simulation = simulation_with(writer.clone());

        assert!(wall_clock_types(&writer).is_empty());
        assert_eq!(simulation.by_ref().count(), 3);
        assert_eq!(
            wall_clock_types(&writer),
            ["simulation_start", "simulation_end"]
        );

        let times = wall_clock_times(&writer);
        assert!(times[0] <= times[1]);

        assert!(simulation.next().is_none());
        drop(simulation);
        assert_eq!(
            wall_clock_types(&writer),
            ["simulation_start", "simulation_end"]
        );
    }

    #[test]
    fn records_wall_clock_end_when_dropped_early() {
        let writer = std::rc::Rc::new(RecordedMetadata::default());
        let mut simulation = simulation_with(writer.clone());

        simulation.next();
        assert_eq!(wall_clock_types(&writer), ["simulation_start"]);

        drop(simulation);
        assert_eq!(
            wall_clock_types(&writer),
            ["simulation_start", "simulation_end"]
        );
    }

    #[test]
    fn does_not_record_wall_clock_times_when_never_started() {
        let writer = std::rc::Rc::new(RecordedMetadata::default());
        drop(simulation_with(writer.clone()));
        assert!(wall_clock_types(&writer).is_empty());
    }
}
