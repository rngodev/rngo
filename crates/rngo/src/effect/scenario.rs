use super::phase::{Phase, PhaseBuilder};
use crate::build::BuildError;
use crate::moment::Moment;
use chrono::{DateTime, FixedOffset, Utc};
use indexmap::IndexMap;

/// Everything a run is parameterized by: the `seed`, the `now` that relative moments resolve against, the default phase (the scenario's own `start` and `end`) and the named phases.
#[derive(Clone, Debug)]
pub struct Scenario {
    pub seed: u64,
    pub now: DateTime<FixedOffset>,
    pub default_phase: Phase,
    pub phases: IndexMap<String, Phase>,
}

impl Scenario {
    pub fn builder() -> ScenarioBuilder {
        ScenarioBuilder::default()
    }

    /// The earliest start among the default and named phases; effects measure their offsets from it so inputs from different phases stay comparable.
    pub fn origin(&self) -> DateTime<FixedOffset> {
        self.phases
            .values()
            .map(|phase| phase.start)
            .fold(self.default_phase.start, |a, b| a.min(b))
    }

    /// The phase `name`, or the default phase for `None`; `None` if no such phase exists.
    pub fn phase(&self, name: Option<&str>) -> Option<&Phase> {
        match name {
            None => Some(&self.default_phase),
            Some(name) => self.phases.get(name),
        }
    }
}

/// Builds a [`Scenario`]; `seed` defaults to 1, `now` to the current time and `start`/`end` to the built-in window.
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

    pub fn set_now(&mut self, now: DateTime<FixedOffset>) -> &mut Self {
        self.now = Some(now);
        self
    }

    pub fn start(mut self, start: Moment) -> Self {
        self.set_start(start);
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.default_phase.set_start(start);
        self
    }

    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.default_phase.set_end(end);
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

    /// Resolves the default phase, then each named phase against it; errors if any phase's `start` isn't before its `end`.
    pub fn build(&self) -> Result<Scenario, Vec<BuildError>> {
        let now = self.now.unwrap_or_else(|| Utc::now().fixed_offset());
        let default_phase = self.default_phase.build(now)?;

        let mut errors = vec![];
        let mut phases = IndexMap::new();
        for (name, builder) in &self.phases {
            match builder.clone().parent(&default_phase).build(now) {
                Ok(phase) => {
                    phases.insert(name.clone(), phase);
                }
                Err(mut e) => errors.append(&mut e),
            }
        }

        if errors.is_empty() {
            Ok(Scenario {
                seed: self.seed.unwrap_or(1),
                now,
                default_phase,
                phases,
            })
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Scenario;
    use crate::moment::Moment;
    use chrono::{DateTime, TimeDelta};

    fn now() -> DateTime<chrono::FixedOffset> {
        DateTime::parse_from_rfc3339("2024-02-01T00:00:00Z").unwrap()
    }

    #[test]
    fn defaults_to_seed_one_and_the_built_in_window() {
        let scenario = Scenario::builder().now(now()).build().unwrap();
        assert_eq!(scenario.seed, 1);
        assert_eq!(scenario.default_phase.end, now());
        assert_eq!(scenario.default_phase.start, now() - TimeDelta::days(30));
    }

    #[test]
    fn named_phases_inherit_what_they_leave_unset() {
        let mut builder = Scenario::builder().now(now()).seed(7);
        builder
            .set_start(Moment::Relative(TimeDelta::days(-10)))
            .with_phase("late", |p| p.start(Moment::Relative(TimeDelta::days(-2))));
        let scenario = builder.build().unwrap();

        let late = scenario.phase(Some("late")).unwrap();
        assert_eq!(scenario.seed, 7);
        assert_eq!(late.start, now() - TimeDelta::days(2));
        assert_eq!(late.end, scenario.default_phase.end);
        assert!(scenario.phase(Some("missing")).is_none());
    }

    #[test]
    fn a_phase_ending_before_it_starts_is_an_error() {
        let mut builder = Scenario::builder().now(now());
        builder.with_phase("backwards", |p| {
            p.end(Moment::Relative(TimeDelta::days(-60)))
        });
        let errors = builder.build().unwrap_err();
        assert!(errors[0].message().contains("backwards"));
    }
}
