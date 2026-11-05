use crate::build::{BuildError, SimulationKey};
use crate::moment::Moment;
use chrono::{DateTime, FixedOffset, Utc};
use std::num::NonZeroU64;

/// A start or end as declared: a fixed moment, or a CEL expression resolved when the simulation builds.
#[derive(Clone, Debug)]
pub enum Bound {
    Moment(Moment),
    Expression(String),
}

impl From<Moment> for Bound {
    fn from(moment: Moment) -> Self {
        Bound::Moment(moment)
    }
}

impl From<DateTime<FixedOffset>> for Bound {
    fn from(timestamp: DateTime<FixedOffset>) -> Self {
        Bound::Moment(timestamp.into())
    }
}

impl From<String> for Bound {
    fn from(expression: String) -> Self {
        Bound::Expression(expression)
    }
}

impl From<&str> for Bound {
    fn from(expression: &str) -> Self {
        Bound::Expression(expression.into())
    }
}

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
    start: Option<Bound>,
    end: Option<Bound>,
    limit: Option<NonZeroU64>,
    now: Option<DateTime<FixedOffset>>,
    default_prototype: Option<Prototype>,
}

impl PrototypeBuilder {
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn start(mut self, start: impl Into<Bound>) -> Self {
        self.set_start(start);
        self
    }

    /// Sets `start` to a [`Bound`]; a string is a CEL expression, parsed at build so it can refer to the default prototype via `simulation`.
    pub fn set_start(&mut self, start: impl Into<Bound>) -> &mut Self {
        self.start = Some(start.into());
        self
    }

    pub fn end(mut self, end: impl Into<Bound>) -> Self {
        self.set_end(end);
        self
    }

    /// Sets `end` to a [`Bound`]; a string is a CEL expression, parsed at build so it can refer to the default prototype via `simulation`.
    pub fn set_end(&mut self, end: impl Into<Bound>) -> &mut Self {
        self.end = Some(end.into());
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

    /// Resolves the prototype; errors if an expression doesn't parse or if `start` isn't before `end`.
    pub fn build(&self) -> Result<Prototype, Vec<BuildError>> {
        let now = self.now.unwrap_or_else(|| Utc::now().fixed_offset());
        let default = self.default_prototype.as_ref();
        let error =
            |_: &str, message: String| BuildError::simulation(SimulationKey::Start, message);
        let mut errors = vec![];
        let start = resolve_bound("start", self.start.as_ref(), now, default, &error)
            .map_err(|e| errors.extend(e));
        let end = resolve_bound("end", self.end.as_ref(), now, default, &error)
            .map_err(|e| errors.extend(e));
        let (Ok(start), Ok(end)) = (start, end) else {
            return Err(errors);
        };

        if let (Some(start), Some(end)) = (start, end)
            && start >= end
        {
            let message = match &self.name {
                Some(name) => format!("prototype \"{name}\": start must be before end"),
                None => "start must be before end".into(),
            };
            return Err(vec![error("start", message)]);
        }

        Ok(Prototype {
            name: self.name.clone(),
            start,
            end,
            limit: self.limit,
        })
    }
}

/// Resolves one declared bound at `now`; `default`'s start and end, when both are set, are what expressions see as `simulation`. Errors are built by `error` from the field name and message.
pub(crate) fn resolve_bound(
    field: &str,
    bound: Option<&Bound>,
    now: DateTime<FixedOffset>,
    default: Option<&Prototype>,
    error: &impl Fn(&str, String) -> BuildError,
) -> Result<Option<DateTime<FixedOffset>>, Vec<BuildError>> {
    let moment = match bound {
        None => return Ok(None),
        Some(Bound::Moment(moment)) => moment.clone(),
        Some(Bound::Expression(expression)) => {
            let simulation = default.and_then(|default| {
                Some((Moment::from(default.start?), Moment::from(default.end?)))
            });
            let mut parser = Moment::parser();
            if let Some((start, end)) = &simulation {
                parser = parser.simulation(start, end);
            }
            parser.parse(field, expression).map_err(|errors| {
                errors
                    .iter()
                    .map(|e| error(field, e.message().to_string()))
                    .collect::<Vec<_>>()
            })?
        }
    };
    Ok(Some(moment.resolve(now)))
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
            .set_end("simulation.end - days(1)");
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
