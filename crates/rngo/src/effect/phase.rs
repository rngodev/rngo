use crate::build::{BuildError, EffectKey, SimulationKey};
use crate::moment::Moment;
use chrono::{DateTime, FixedOffset, Utc};
use std::num::NonZeroU64;

/// A time window and an input cap, each optional; an effect takes the parts it doesn't set itself from its phase.
#[derive(Clone, Debug, Default)]
pub struct Phase {
    /// The phase's name; `None` for the default phase and for an effect's own window.
    pub name: Option<String>,
    pub start: Option<DateTime<FixedOffset>>,
    pub end: Option<DateTime<FixedOffset>>,
    pub limit: Option<NonZeroU64>,
}

impl Phase {
    pub fn builder() -> PhaseBuilder {
        PhaseBuilder::default()
    }
}

/// Builds a [`Phase`]; whatever is left unset stays unset.
#[derive(Clone, Debug, Default)]
pub struct PhaseBuilder {
    name: Option<String>,
    start: Option<Moment>,
    end: Option<Moment>,
    start_expression: Option<String>,
    end_expression: Option<String>,
    limit: Option<NonZeroU64>,
    now: Option<DateTime<FixedOffset>>,
    default_phase: Option<Phase>,
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

    /// Sets `start` from an expression, parsed at build; overrides `start`.
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

    /// Sets `end` from an expression, parsed at build; overrides `end`.
    pub fn set_end_expression(&mut self, expression: String) -> &mut Self {
        self.end_expression = Some(expression);
        self
    }

    pub fn limit(mut self, limit: NonZeroU64) -> Self {
        self.set_limit(limit);
        self
    }

    pub fn set_limit(&mut self, limit: NonZeroU64) -> &mut Self {
        self.limit = Some(limit);
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

    pub fn default_phase(mut self, phase: &Phase) -> Self {
        self.set_default_phase(phase);
        self
    }

    /// Sets the default phase, whose `start` and `end` expressions see as `simulation`; without it, or if it lacks either, expressions can't refer to `simulation`.
    pub fn set_default_phase(&mut self, phase: &Phase) -> &mut Self {
        self.default_phase = Some(phase.clone());
        self
    }

    /// Makes this the window of effect `key`, so errors are reported against the effect.
    pub fn for_effect(mut self, key: impl Into<String>) -> Self {
        self.effect = Some(key.into());
        self
    }

    /// Resolves the phase; errors if an expression doesn't parse or, outside an effect, if `start` isn't before `end`.
    pub fn build(&self) -> Result<Phase, Vec<BuildError>> {
        let now = self.now.unwrap_or_else(|| Utc::now().fixed_offset());
        let simulation = self
            .default_phase
            .as_ref()
            .and_then(|phase| Some((Moment::from(phase.start?), Moment::from(phase.end?))));
        let mut parser = Moment::parser();
        if let Some((start, end)) = &simulation {
            parser = parser.simulation(start, end);
        }
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

        if self.effect.is_none()
            && let (Some(start), Some(end)) = (start, end)
            && start >= end
        {
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
            start,
            end,
            limit: self.limit,
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
    use crate::moment::Moment;
    use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
    use std::num::NonZeroU64;

    fn now() -> DateTime<FixedOffset> {
        let now = Utc::now().fixed_offset();
        now - TimeDelta::nanoseconds(now.timestamp_subsec_nanos().into())
    }

    #[test]
    fn leaves_unset_parts_unset() {
        let phase = Phase::builder().build().unwrap();
        assert!(phase.start.is_none() && phase.end.is_none() && phase.limit.is_none());
    }

    #[test]
    fn resolves_moments_expressions_and_limit() {
        let mut builder = Phase::builder().limit(NonZeroU64::new(3).unwrap());
        builder
            .set_start(Moment::Relative(TimeDelta::days(-2)))
            .set_end_expression("simulation.end - days(1)".into());
        let now = now();
        let phase = builder
            .set_now(now)
            .set_default_phase(&Phase {
                start: Some(now - TimeDelta::days(30)),
                end: Some(now),
                ..Phase::default()
            })
            .build()
            .unwrap();

        assert_eq!(phase.start, Some(now - TimeDelta::days(2)));
        assert_eq!(phase.end, Some(now - TimeDelta::days(1)));
        assert_eq!(phase.limit, NonZeroU64::new(3));
    }

    #[test]
    fn a_phase_ending_before_it_starts_is_an_error() {
        let errors = Phase::builder()
            .name("backwards")
            .start(Moment::Relative(TimeDelta::days(-1)))
            .end(Moment::Relative(TimeDelta::days(-60)))
            .build()
            .unwrap_err();
        assert!(errors[0].message().contains("backwards"));
    }
}
