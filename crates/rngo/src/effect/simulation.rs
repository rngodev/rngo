use crate::effect::bound::Bound;
use crate::effect::prototype::{Prototype, PrototypeBuilder};
use crate::effect::{Effect, EffectBuilder, EffectContext, Step};
use crate::error::{BuildError, BuildErrors, SimulationKey};
use crate::log::SimpleLog;
use crate::moment::Moment;
use crate::{LogReader, SimulationError};
use indexmap::IndexMap;
use jiff::{SignedDuration, Timestamp};
use std::sync::Arc;

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

/// Yields each effect's steps in time order; an `Err` ends the effect that failed, and callers should stop there.
impl Iterator for Simulation {
    type Item = Result<Step, SimulationError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.limit.is_some_and(|limit| self.emitted >= limit) {
            return None;
        }

        self.effects
            .sort_unstable_by_key(|e| e.next_offset().unwrap_or(i64::MAX));

        let item = self.effects.first_mut()?.next()?;
        if let Ok(Step::Input(_)) = item {
            self.emitted += 1;
        }
        Some(item)
    }
}

/// A [`Simulation`] that owns an in-memory log and writes each input it yields to it.
#[derive(Debug)]
pub struct StandaloneSimulation {
    simulation: Simulation,
    log: Arc<SimpleLog>,
}

impl StandaloneSimulation {
    /// The log the inputs are written to, which the simulation's effects read from.
    pub fn log(&self) -> &Arc<SimpleLog> {
        &self.log
    }
}

impl Iterator for StandaloneSimulation {
    type Item = Result<Step, SimulationError>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.simulation.next()?;
        if let Ok(Step::Input(input)) = &item {
            self.log.insert_input(input.clone());
        }
        Some(item)
    }
}

#[derive(Debug)]
pub struct SimulationBuilder {
    log_reader: Option<Arc<dyn LogReader>>,
    effect_builders: Vec<EffectBuilder>,
    seed: Option<u64>,
    now: Option<Timestamp>,
    default_prototype: PrototypeBuilder,
    prototypes: IndexMap<String, PrototypeBuilder>,
    limit: Option<u64>,
}

impl SimulationBuilder {
    fn new() -> Self {
        SimulationBuilder {
            log_reader: None,
            effect_builders: vec![],
            seed: None,
            now: None,
            default_prototype: PrototypeBuilder::default()
                .start(Moment::Relative(SignedDuration::from_hours(24 * -30)))
                .end(Moment::Relative(SignedDuration::ZERO)),
            prototypes: IndexMap::new(),
            limit: None,
        }
    }

    pub fn log_reader<T: LogReader + 'static>(mut self, reader: Arc<T>) -> Self {
        self.log_reader = Some(reader as Arc<dyn LogReader>);
        self
    }

    /// Seeds every random draw, so a run with the same seed, `now` and spec repeats exactly; defaults to 1.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// The instant relative bounds are measured from; defaults to the current time at build. Set it to make runs reproducible.
    pub fn now(mut self, now: Timestamp) -> Self {
        self.now = Some(now);
        self
    }

    /// Sets the default prototype's start; defaults to 30 days before now.
    pub fn start(mut self, start: impl Into<Bound>) -> Self {
        self.default_prototype = self.default_prototype.start(start);
        self
    }

    /// Sets the default prototype's end; defaults to now.
    pub fn end(mut self, end: impl Into<Bound>) -> Self {
        self.default_prototype = self.default_prototype.end(end);
        self
    }

    /// Defines a named prototype; `f` configures its builder.
    pub fn prototype(
        mut self,
        name: impl Into<String>,
        f: impl FnOnce(PrototypeBuilder) -> PrototypeBuilder,
    ) -> Self {
        let name = name.into();
        self.prototypes.insert(
            name.clone(),
            f(PrototypeBuilder::default().name(name.clone())),
        );
        self
    }

    /// The most inputs the whole simulation produces; skipped attempts don't count. Must be at least 1.
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Adds an effect; `f` configures its builder.
    pub fn effect(
        mut self,
        key: impl Into<String>,
        f: impl FnOnce(EffectBuilder) -> EffectBuilder,
    ) -> Self {
        self.effect_builders.push(f(Effect::builder(key)));
        self
    }

    /// Builds a [`StandaloneSimulation`] over a new in-memory log, replacing any log reader set.
    pub fn standalone(self) -> Result<StandaloneSimulation, BuildErrors> {
        let log = Arc::new(SimpleLog::new());
        let simulation = self.log_reader(log.clone()).build()?;
        Ok(StandaloneSimulation { simulation, log })
    }

    pub fn build(self) -> Result<Simulation, BuildErrors> {
        let mut errors = vec![];

        if self.limit == Some(0) {
            errors.push(BuildError::simulation(
                SimulationKey::Limit,
                "limit must be at least 1",
            ));
        }

        let Some(log_reader) = self.log_reader else {
            errors.push(BuildError::simulation(
                SimulationKey::LogReader,
                "log_reader was not set",
            ));
            return Err(errors.into());
        };

        let now = self.now.unwrap_or_else(Timestamp::now);
        let default_prototype = match self.default_prototype.clone().now(now).build() {
            Ok(prototype) => prototype,
            Err(e) => {
                errors.extend(e);
                return Err(errors.into());
            }
        };
        let (Some(start), Some(_)) = (default_prototype.start, default_prototype.end) else {
            errors.push(BuildError::simulation(
                SimulationKey::Start,
                "the simulation must have a start and an end",
            ));
            return Err(errors.into());
        };

        let mut prototypes = IndexMap::new();
        for (name, builder) in &self.prototypes {
            match builder
                .clone()
                .now(now)
                .prototype(&default_prototype)
                .build()
            {
                Ok(prototype) => {
                    prototypes.insert(name.clone(), prototype);
                }
                Err(e) => errors.extend(e),
            }
        }
        if !errors.is_empty() {
            return Err(errors.into());
        }

        let empty_prototype = Prototype::default();

        let mut effects = vec![];
        for effect_builder in self.effect_builders {
            let prototype = match effect_builder.prototype_key.as_deref() {
                Some(key) => prototypes.get(key).unwrap_or(&empty_prototype),
                None => &default_prototype,
            };
            let context = EffectContext {
                now,
                seed: self.seed.unwrap_or(1),
                prototype,
                log_reader: log_reader.clone(),
            };

            match effect_builder.build(context) {
                Ok(effect) => effects.push(effect),
                Err(e) => errors.extend(e),
            }
        }

        let origin = effects
            .iter()
            .map(|effect| effect.start)
            .fold(start, Ord::min);
        for effect in &mut effects {
            if let Err(e) = effect.set_origin(origin) {
                errors.extend(e);
            }
        }

        if errors.is_empty() {
            Ok(Simulation {
                effects,
                limit: self.limit,
                emitted: 0,
            })
        } else {
            Err(errors.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::effect::generator::{
        Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext, GeneratorMetadata,
        GeneratorResult,
    };
    use crate::error::{BuildError, BuildErrors};

    #[derive(Debug, Default)]
    struct AlternatingGenerator {
        calls: u32,
    }

    impl Generator for AlternatingGenerator {
        fn next(&mut self, _context: &GeneratorContext) -> GeneratorResult {
            self.calls += 1;
            if self.calls % 2 == 1 {
                GeneratorResult {
                    value: Some(serde_json::Value::Null),
                    metadata: vec![],
                }
            } else {
                GeneratorResult {
                    value: None,
                    metadata: vec![GeneratorMetadata {
                        kind: "error".into(),
                        attribute: None,
                        data: Some(serde_json::json!({ "message": "boom" })),
                    }],
                }
            }
        }
    }

    #[derive(Debug)]
    struct AlternatingGeneratorBuilder;

    impl GeneratorBuilder for AlternatingGeneratorBuilder {
        fn build(
            self: Box<Self>,
            _visitor: GeneratorBuildVisitor,
        ) -> Result<Box<dyn Generator>, BuildErrors> {
            Ok(Box::new(AlternatingGenerator::default()))
        }
    }

    #[test]
    fn limit_counts_only_inputs() {
        let mut simulation_builder =
            super::Simulation::builder().log_reader(std::sync::Arc::new(crate::SimpleLog::new()));

        simulation_builder = simulation_builder.effect("alternating", |e| {
            e.trigger_hertz(1000.0)
                .generator(AlternatingGeneratorBuilder)
        });

        let items: Vec<_> = simulation_builder.limit(5).build().unwrap().collect();

        assert_eq!(
            items
                .iter()
                .filter(|item| matches!(item, Ok(super::Step::Input(_))))
                .count(),
            5,
            "limit should cap real inputs"
        );
        assert!(
            items
                .iter()
                .any(|item| matches!(item, Ok(super::Step::Skipped(_)))),
            "skipped attempts should not count toward the cap"
        );
    }

    #[test]
    fn build_without_a_log_reader_is_an_error() {
        let errors = super::Simulation::builder().build().unwrap_err();

        assert!(matches!(
            errors.as_slice(),
            [BuildError::Simulation {
                key: crate::SimulationKey::LogReader,
                ..
            }]
        ));
    }

    #[test]
    fn standalone_logs_inputs_so_later_effects_can_read_them() {
        use crate::LogReader;

        let simulation_builder = super::Simulation::builder()
            .effect("upstream", |e| {
                e.trigger_hertz(1.0)
                    .limit(3)
                    .generator(AlternatingGeneratorBuilder)
            })
            .effect("downstream", |e| {
                e.trigger_effect("upstream")
                    .generator(AlternatingGeneratorBuilder)
            });

        let mut simulation = simulation_builder.standalone().unwrap();
        let yielded = simulation.by_ref().flatten().count();

        assert!(yielded > 0);
        assert!(simulation.log().last().is_some());
    }
}
