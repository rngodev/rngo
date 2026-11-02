use super::scenario::ScenarioData;
use crate::build::{BuildError, EffectKey, SimulationKey};
use crate::moment::Moment;
use chrono::{DateTime, FixedOffset, TimeDelta};
use std::rc::Rc;

/// A time window with its `start` and `end` resolved to instants.
#[derive(Clone, Debug)]
pub struct Phase {
    /// The phase's name; `None` for the default phase.
    pub name: Option<String>,
    pub scenario: Rc<ScenarioData>,
    /// The phase this one falls back to and, for an effect's window, must stay within.
    pub parent: Option<Rc<Phase>>,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
}

impl Phase {
    pub fn builder() -> PhaseBuilder {
        PhaseBuilder::default()
    }
}

/// Builds a [`Phase`]; a `start` or `end` left unset comes from the parent, then from the built-in default (30 days before now to now).
#[derive(Clone, Debug, Default)]
pub struct PhaseBuilder {
    name: Option<String>,
    pub start: Option<Moment>,
    end: Option<Moment>,
    start_expression: Option<String>,
    end_expression: Option<String>,
    scenario: Option<Rc<ScenarioData>>,
    parent: Option<Rc<Phase>>,
    effect: Option<String>,
}

impl PhaseBuilder {
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn start(mut self, start: Moment) -> Self {
        self.set_start(start);
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.start = Some(start);
        self
    }

    /// Sets `start` from an expression, parsed at build with the parent's bounds as `simulation`; overrides `start`.
    pub fn set_start_expression(&mut self, expression: String) -> &mut Self {
        self.start_expression = Some(expression);
        self
    }

    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.end = Some(end);
        self
    }

    /// Sets `end` from an expression, parsed at build with the parent's bounds as `simulation`; overrides `end`.
    pub fn set_end_expression(&mut self, expression: String) -> &mut Self {
        self.end_expression = Some(expression);
        self
    }

    /// Makes this the window of effect `key`: it must lie within the parent, and errors are reported against the effect.
    pub fn for_effect(mut self, key: impl Into<String>) -> Self {
        self.effect = Some(key.into());
        self
    }

    pub fn scenario(mut self, scenario: Rc<ScenarioData>) -> Self {
        self.set_scenario(scenario);
        self
    }

    /// Sets the scenario the phase belongs to, whose `now` relative moments resolve against; defaults to the parent's.
    pub fn set_scenario(&mut self, scenario: Rc<ScenarioData>) -> &mut Self {
        self.scenario = Some(scenario);
        self
    }

    pub fn parent(mut self, parent: Rc<Phase>) -> Self {
        self.set_parent(parent);
        self
    }

    /// Sets the phase that supplies whichever of `start` and `end` this one leaves unset.
    pub fn set_parent(&mut self, parent: Rc<Phase>) -> &mut Self {
        self.parent = Some(parent);
        self
    }

    /// Resolves the phase; errors if there is neither a scenario nor a parent, if an expression doesn't parse, and otherwise if `start` isn't before `end` (or, for an effect, if the window leaves the parent).
    pub fn build(&self) -> Result<Phase, Vec<BuildError>> {
        let parent = self.parent.as_ref();
        let Some(scenario) = self
            .scenario
            .as_ref()
            .or_else(|| parent.map(|parent| &parent.scenario))
        else {
            return Err(vec![self.error(
                "start",
                "scenario must be set via set_scenario() or a parent",
            )]);
        };
        let now = scenario.now;
        let (bounds_start, bounds_end) = match parent {
            Some(parent) => (parent.start, parent.end),
            None => (now - TimeDelta::days(30), now),
        };
        let (start_moment, end_moment) = (Moment::from(bounds_start), Moment::from(bounds_end));
        let parser = Moment::parser().simulation(&start_moment, &end_moment);
        let mut errors = vec![];

        let mut bound = |field: &str, moment: &Option<Moment>, expression: &Option<String>| {
            let moment = match expression {
                Some(expression) => match parser.parse(field, expression) {
                    Ok(moment) => Some(moment),
                    Err(e) => {
                        errors.extend(e.iter().map(|e| self.error(field, e.message())));
                        return None;
                    }
                },
                None => moment.clone(),
            };
            Some(moment.map(|moment| moment.resolve(now)))
        };
        let start = bound("start", &self.start, &self.start_expression);
        let end = bound("end", &self.end, &self.end_expression);
        let (Some(start), Some(end)) = (start, end) else {
            return Err(errors);
        };
        let start = start.unwrap_or(bounds_start);
        let end = end.unwrap_or(bounds_end);

        if self.effect.is_some() {
            let bounds = match parent.and_then(|parent| parent.name.as_ref()) {
                Some(name) => format!("phase \"{name}\""),
                None => "simulation".into(),
            };
            if start < bounds_start {
                errors.push(self.error("start", format!("start cannot be before {bounds} start")));
            }
            if end > bounds_end {
                errors.push(self.error("end", format!("end cannot be after {bounds} end")));
            }
        } else if start >= end {
            let message = match &self.name {
                Some(name) => format!("phase \"{name}\": start must be before end"),
                None => "start must be before end".into(),
            };
            errors.push(self.error("start", message));
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(Phase {
            name: self.name.clone(),
            scenario: Rc::clone(scenario),
            parent: parent.cloned(),
            start,
            end,
        })
    }

    fn error(&self, field: &str, message: impl Into<String>) -> BuildError {
        match &self.effect {
            Some(key) => {
                let key_kind = if field == "start" {
                    EffectKey::Start
                } else {
                    EffectKey::End
                };
                BuildError::effect(key.clone(), key_kind, message)
            }
            None => BuildError::simulation(SimulationKey::Start, message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Phase;
    use crate::effect::scenario::ScenarioData;
    use crate::moment::Moment;
    use chrono::{DateTime, FixedOffset, TimeDelta};
    use std::rc::Rc;

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2024-02-01T00:00:00Z").unwrap()
    }

    fn scenario(seed: u64) -> Rc<ScenarioData> {
        Rc::new(ScenarioData {
            seed,
            now: now(),
            origin: now(),
        })
    }

    #[test]
    fn defaults_to_the_built_in_window() {
        let phase = Phase::builder().scenario(scenario(1)).build().unwrap();
        assert_eq!(phase.end, now());
        assert_eq!(phase.start, now() - TimeDelta::days(30));
    }

    #[test]
    fn a_child_inherits_what_it_leaves_unset() {
        let scenario = scenario(7);
        let default = Rc::new(
            Phase::builder()
                .start(Moment::Relative(TimeDelta::days(-10)))
                .scenario(Rc::clone(&scenario))
                .build()
                .unwrap(),
        );
        let late = Phase::builder()
            .name("late")
            .start(Moment::Relative(TimeDelta::days(-2)))
            .parent(default)
            .build()
            .unwrap();

        assert_eq!(late.scenario.seed, 7);
        assert_eq!(late.start, now() - TimeDelta::days(2));
        assert_eq!(late.end, now());
        assert_eq!(late.parent.unwrap().start, now() - TimeDelta::days(10));
    }

    #[test]
    fn a_phase_ending_before_it_starts_is_an_error() {
        let scenario = scenario(1);
        let default = Rc::new(Phase::builder().scenario(scenario).build().unwrap());
        let errors = Phase::builder()
            .name("backwards")
            .end(Moment::Relative(TimeDelta::days(-60)))
            .parent(default)
            .build()
            .unwrap_err();
        assert!(errors[0].message().contains("backwards"));
    }
}
