use crate::build::{BuildError, EffectKey, SimulationKey};
use crate::moment::Moment;
use chrono::{DateTime, FixedOffset, Utc};
use std::num::NonZeroU64;

/// A time window and an input cap, each optional; an effect takes the parts it doesn't set itself from its prototype.
#[derive(Clone, Debug, Default)]
pub struct Prototype {
    /// The prototype's name; `None` for the default prototype and for an effect's own window.
    pub name: Option<String>,
    pub start: Option<DateTime<FixedOffset>>,
    pub end: Option<DateTime<FixedOffset>>,
    pub limit: Option<NonZeroU64>,
}

impl Prototype {
    pub fn builder() -> PrototypeBuilder {
        PrototypeBuilder::default()
    }
}

/// Builds a [`Prototype`]; whatever is left unset stays unset.
#[derive(Clone, Debug, Default)]
pub struct PrototypeBuilder {
    name: Option<String>,
    start: Option<Moment>,
    end: Option<Moment>,
    start_expression: Option<String>,
    end_expression: Option<String>,
    limit: Option<NonZeroU64>,
    now: Option<DateTime<FixedOffset>>,
    default_prototype: Option<Prototype>,
    effect: Option<String>,
}

impl PrototypeBuilder {
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

    pub fn default_prototype(mut self, prototype: &Prototype) -> Self {
        self.set_default_prototype(prototype);
        self
    }

    /// Sets the default prototype, whose `start` and `end` expressions see as `simulation`; without it, or if it lacks either, expressions can't refer to `simulation`.
    pub fn set_default_prototype(&mut self, prototype: &Prototype) -> &mut Self {
        self.default_prototype = Some(prototype.clone());
        self
    }

    /// Makes this the window of effect `key`, so errors are reported against the effect.
    pub fn for_effect(mut self, key: impl Into<String>) -> Self {
        self.effect = Some(key.into());
        self
    }

    /// Resolves the prototype; errors if an expression doesn't parse or, outside an effect, if `start` isn't before `end`.
    pub fn build(&self) -> Result<Prototype, Vec<BuildError>> {
        let now = self.now.unwrap_or_else(|| Utc::now().fixed_offset());
        let simulation = self.default_prototype.as_ref().and_then(|prototype| {
            Some((Moment::from(prototype.start?), Moment::from(prototype.end?)))
        });
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
                Some(name) => format!("prototype \"{name}\": start must be before end"),
                None => "start must be before end".into(),
            };
            errors.push(self.error("start", message));
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(Prototype {
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
    use super::Prototype;
    use crate::moment::Moment;
    use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
    use std::num::NonZeroU64;

    fn now() -> DateTime<FixedOffset> {
        let now = Utc::now().fixed_offset();
        now - TimeDelta::nanoseconds(now.timestamp_subsec_nanos().into())
    }

    #[test]
    fn leaves_unset_parts_unset() {
        let prototype = Prototype::builder().build().unwrap();
        assert!(prototype.start.is_none() && prototype.end.is_none() && prototype.limit.is_none());
    }

    #[test]
    fn resolves_moments_expressions_and_limit() {
        let mut builder = Prototype::builder().limit(NonZeroU64::new(3).unwrap());
        builder
            .set_start(Moment::Relative(TimeDelta::days(-2)))
            .set_end_expression("simulation.end - days(1)".into());
        let now = now();
        let prototype = builder
            .set_now(now)
            .set_default_prototype(&Prototype {
                start: Some(now - TimeDelta::days(30)),
                end: Some(now),
                ..Prototype::default()
            })
            .build()
            .unwrap();

        assert_eq!(prototype.start, Some(now - TimeDelta::days(2)));
        assert_eq!(prototype.end, Some(now - TimeDelta::days(1)));
        assert_eq!(prototype.limit, NonZeroU64::new(3));
    }

    #[test]
    fn a_prototype_ending_before_it_starts_is_an_error() {
        let errors = Prototype::builder()
            .name("backwards")
            .start(Moment::Relative(TimeDelta::days(-1)))
            .end(Moment::Relative(TimeDelta::days(-60)))
            .build()
            .unwrap_err();
        assert!(errors[0].message().contains("backwards"));
    }
}
