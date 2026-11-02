use crate::RunLogReader;
use crate::build::{BuildError, SimulationKey};
use crate::effect::phase::{Phase, PhaseBuilder};
use crate::effect::{Effect, EffectBuilder, Input, SkippedInput};
use crate::log::SimpleEventRunLog;
use crate::moment::Moment;
use chrono::{DateTime, FixedOffset, Utc};
use indexmap::IndexMap;
use std::rc::Rc;

/// What a run is parameterized by besides time windows: the `seed`, the `now` that relative moments resolve against and the `origin` (the earliest phase start) that offsets are measured from, so inputs from different phases stay comparable.
#[derive(Clone, Debug)]
pub struct SimulationData {
    pub seed: u64,
    pub now: DateTime<FixedOffset>,
    pub origin: DateTime<FixedOffset>,
}

struct Phases {
    default: Rc<Phase>,
    named: IndexMap<String, Rc<Phase>>,
}

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
        if item.is_ok() {
            self.emitted += 1;
        }
        Some(item)
    }
}

/// A [`Simulation`] that owns an in-memory log and writes each input it yields to it.
#[derive(Debug)]
pub struct StandaloneSimulation {
    simulation: Simulation,
    log: Rc<SimpleEventRunLog>,
}

impl StandaloneSimulation {
    /// The log the inputs are written to, which the simulation's effects read from.
    pub fn run_log(&self) -> &Rc<SimpleEventRunLog> {
        &self.log
    }
}

impl Iterator for StandaloneSimulation {
    type Item = Result<Input, SkippedInput>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.simulation.next()?;
        if let Ok(input) = &item {
            self.log.insert_input(input.clone());
        }
        Some(item)
    }
}

#[derive(Debug)]
pub struct SimulationBuilder {
    run_log_reader: Option<Rc<dyn RunLogReader>>,
    effect_builders: Vec<EffectBuilder>,
    seed: Option<u64>,
    now: Option<DateTime<FixedOffset>>,
    default_phase: PhaseBuilder,
    phases: IndexMap<String, PhaseBuilder>,
    limit: Option<u64>,
}

impl SimulationBuilder {
    fn new() -> Self {
        SimulationBuilder {
            run_log_reader: None,
            effect_builders: vec![],
            seed: None,
            now: None,
            default_phase: PhaseBuilder::default(),
            phases: IndexMap::new(),
            limit: None,
        }
    }

    pub fn run_log_reader<T: RunLogReader + 'static>(mut self, reader: Rc<T>) -> Self {
        self.run_log_reader = Some(reader as Rc<dyn RunLogReader>);
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.set_seed(seed);
        self
    }

    /// Sets the seed; defaults to 1.
    pub fn set_seed(&mut self, seed: u64) -> &mut Self {
        self.seed = Some(seed);
        self
    }

    pub fn now(mut self, now: DateTime<FixedOffset>) -> Self {
        self.set_now(now);
        self
    }

    /// Sets the instant relative moments resolve against; defaults to the current time at build.
    pub fn set_now(&mut self, now: DateTime<FixedOffset>) -> &mut Self {
        self.now = Some(now);
        self
    }

    /// Sets the default phase's start; defaults to 30 days before now.
    pub fn start(mut self, start: Moment) -> Self {
        self.set_start(start);
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.default_phase.set_start(start);
        self
    }

    /// Sets the default phase's end; defaults to now.
    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.default_phase.set_end(end);
        self
    }

    pub fn phase(mut self, name: &str, f: impl FnOnce(PhaseBuilder) -> PhaseBuilder) -> Self {
        self.with_phase(name, f);
        self
    }

    /// Defines a named phase; `f` configures its builder, and whatever it leaves unset comes from the default phase.
    pub fn with_phase(
        &mut self,
        name: &str,
        f: impl FnOnce(PhaseBuilder) -> PhaseBuilder,
    ) -> &mut Self {
        self.phases
            .insert(name.into(), f(PhaseBuilder::default().name(name)));
        self
    }

    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Adds an effect; `f` configures its builder.
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

    pub fn effect(mut self, key: &str, f: impl FnOnce(EffectBuilder) -> EffectBuilder) -> Self {
        self.with_effect(key, f);
        self
    }

    /// Builds a [`StandaloneSimulation`] over a new in-memory log, replacing any run log reader set.
    pub fn standalone(self) -> Result<StandaloneSimulation, Vec<BuildError>> {
        let log = SimpleEventRunLog::new();
        let simulation = self.run_log_reader(log.clone()).build()?;
        Ok(StandaloneSimulation { simulation, log })
    }

    pub fn build(self) -> Result<Simulation, Vec<BuildError>> {
        let mut errors = vec![];

        let Some(run_log_reader) = self.run_log_reader else {
            errors.push(BuildError::simulation(
                SimulationKey::RunLogReader,
                "run_log_reader was not set",
            ));
            return Err(errors);
        };

        let now = self.now.unwrap_or_else(|| Utc::now().fixed_offset());
        let seed = self.seed.unwrap_or(1);
        let provisional = Rc::new(SimulationData {
            seed,
            now,
            origin: now,
        });
        let phases = build_phases(&self.default_phase, &self.phases, &provisional)?;
        let origin = phases
            .named
            .values()
            .map(|phase| phase.start)
            .fold(phases.default.start, |a, b| a.min(b));
        let data = Rc::new(SimulationData { seed, now, origin });
        let phases = build_phases(&self.default_phase, &self.phases, &data)?;

        let mut effects = vec![];
        for mut effect_builder in self.effect_builders {
            effect_builder.set_event_run_log(run_log_reader.clone());
            let phase = effect_builder
                .phase
                .as_ref()
                .and_then(|name| phases.named.get(name))
                .unwrap_or(&phases.default);

            match effect_builder.build(phase) {
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

/// Builds the default phase and each named phase (with the default as its parent), so every phase is validated, used or not.
fn build_phases(
    default: &PhaseBuilder,
    named: &IndexMap<String, PhaseBuilder>,
    data: &Rc<SimulationData>,
) -> Result<Phases, Vec<BuildError>> {
    let default = Rc::new(default.clone().simulation(Rc::clone(data)).build()?);

    let mut errors = vec![];
    let mut phases = IndexMap::new();
    for (name, builder) in named {
        match builder
            .clone()
            .simulation(Rc::clone(data))
            .parent(Rc::clone(&default))
            .build()
        {
            Ok(phase) => {
                phases.insert(name.clone(), Rc::new(phase));
            }
            Err(mut e) => errors.append(&mut e),
        }
    }

    if errors.is_empty() {
        Ok(Phases {
            default,
            named: phases,
        })
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use crate::build::BuildError;
    use crate::effect::generator::{
        Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext, GeneratorResult,
        Metadata,
    };

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
    struct AlternatingGeneratorBuilder;

    impl GeneratorBuilder for AlternatingGeneratorBuilder {
        fn build(
            &self,
            _visitor: GeneratorBuildVisitor,
        ) -> Result<Box<dyn Generator>, Vec<BuildError>> {
            Ok(Box::new(AlternatingGenerator::default()))
        }
    }

    #[test]
    fn limit_counts_only_inputs() {
        let mut simulation_builder =
            super::Simulation::builder().run_log_reader(crate::SimpleEventRunLog::new());

        simulation_builder.with_effect("alternating", |e| {
            e.trigger_hertz(1000.0)
                .generator(AlternatingGeneratorBuilder)
        });

        let items: Vec<_> = simulation_builder.limit(5).build().unwrap().collect();

        assert_eq!(
            items.iter().filter(|item| item.is_ok()).count(),
            5,
            "limit should cap real inputs"
        );
        assert!(
            items.iter().any(|item| item.is_err()),
            "skipped attempts should not count toward the cap"
        );
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

    #[test]
    fn standalone_logs_inputs_so_later_effects_can_read_them() {
        use crate::RunLogReader;

        let mut simulation_builder = super::Simulation::builder();
        simulation_builder
            .with_effect("upstream", |e| {
                e.trigger_hertz(1.0)
                    .limit(std::num::NonZeroU64::new(3).unwrap())
                    .generator(AlternatingGeneratorBuilder)
            })
            .with_effect("downstream", |e| {
                e.trigger_effect("upstream".into())
                    .generator(AlternatingGeneratorBuilder)
            });

        let mut simulation = simulation_builder.standalone().unwrap();
        let yielded = simulation.by_ref().flatten().count();

        assert!(yielded > 0);
        assert!(simulation.run_log().last().is_some());
    }
}
