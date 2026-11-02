use super::phase::{Phase, PhaseBuilder};
use crate::build::BuildError;
use crate::moment::Moment;
use chrono::{DateTime, FixedOffset, Utc};
use indexmap::IndexMap;
use std::rc::Rc;

/// A built scenario: its [`ScenarioData`], default phase and named phases, each of which refers back to the data.
#[derive(Clone, Debug)]
pub struct Scenario {
    pub data: Rc<ScenarioData>,
    default_phase: Rc<Phase>,
    phases: IndexMap<String, Rc<Phase>>,
}

impl Scenario {
    pub fn builder() -> ScenarioBuilder {
        ScenarioBuilder::default()
    }

    pub fn default_phase(&self) -> &Rc<Phase> {
        &self.default_phase
    }

    /// The named phase `name`; `None` if there is no such phase.
    pub fn get_phase(&self, name: &str) -> Option<&Rc<Phase>> {
        self.phases.get(name)
    }

    /// The named phase `key`, or the default phase if there is no such phase.
    pub fn phase(&self, key: &str) -> &Rc<Phase> {
        self.phases.get(key).unwrap_or(&self.default_phase)
    }
}

/// What a run is parameterized by besides time windows: the `seed`, the `now` that relative moments resolve against and the `origin` (the earliest phase start) that offsets are measured from, so inputs from different phases stay comparable.
#[derive(Clone, Debug)]
pub struct ScenarioData {
    pub seed: u64,
    pub now: DateTime<FixedOffset>,
    pub origin: DateTime<FixedOffset>,
}

/// Builds a [`Scenario`] along with its phases; `seed` defaults to 1, and the scenario's own `start` and `end` define the default phase.
#[derive(Clone, Debug, Default)]
pub struct ScenarioBuilder {
    seed: Option<u64>,
    now: Option<DateTime<FixedOffset>>,
    default_phase: PhaseBuilder,
    phases: IndexMap<String, PhaseBuilder>,
}

impl ScenarioBuilder {
    pub fn seed(mut self, seed: u64) -> Self {
        self.set_seed(seed);
        self
    }

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

    /// Builds the default phase and each named phase (with the default as its parent); the scenario's origin is the earliest start among them.
    pub fn build(&self) -> Result<Scenario, Vec<BuildError>> {
        let now = self.now.unwrap_or_else(|| Utc::now().fixed_offset());
        let seed = self.seed.unwrap_or(1);
        let provisional = Rc::new(ScenarioData {
            seed,
            now,
            origin: now,
        });
        let (default_phase, phases) = self.build_phases(&provisional)?;
        let origin = phases
            .values()
            .map(|phase| phase.start)
            .fold(default_phase.start, |a, b| a.min(b));

        let data = Rc::new(ScenarioData { seed, now, origin });
        let (default_phase, phases) = self.build_phases(&data)?;

        Ok(Scenario {
            data,
            default_phase,
            phases,
        })
    }

    #[allow(clippy::type_complexity)]
    fn build_phases(
        &self,
        data: &Rc<ScenarioData>,
    ) -> Result<(Rc<Phase>, IndexMap<String, Rc<Phase>>), Vec<BuildError>> {
        let default_phase = Rc::new(self.default_phase.build(data, None)?);

        let mut errors = vec![];
        let mut phases = IndexMap::new();
        for (name, builder) in &self.phases {
            match builder.build(data, Some(&default_phase)) {
                Ok(phase) => {
                    phases.insert(name.clone(), Rc::new(phase));
                }
                Err(mut e) => errors.append(&mut e),
            }
        }

        if errors.is_empty() {
            Ok((default_phase, phases))
        } else {
            Err(errors)
        }
    }
}
