use crate::build::{BuildError, SimulationKey};
use crate::effect::bound::{Bound, BoundKind};
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
    /// The key of the channel effects on this prototype write to unless they set their own.
    pub channel: Option<String>,
}

impl Prototype {
    pub fn builder() -> PrototypeBuilder {
        PrototypeBuilder::default()
    }
}

/// Builds a prototype; whatever is left unset stays unset.
#[derive(Clone, Debug, Default)]
pub struct PrototypeBuilder {
    name: Option<String>,
    start: Option<Bound>,
    end: Option<Bound>,
    limit: Option<NonZeroU64>,
    channel: Option<String>,
    now: Option<DateTime<FixedOffset>>,
    prototype: Option<Prototype>,
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
        let start: Bound = start.into();
        self.start = Some(start.of_kind(BoundKind::Start));
        self
    }

    pub fn end(mut self, end: impl Into<Bound>) -> Self {
        self.set_end(end);
        self
    }

    /// Sets `end` to a [`Bound`]; a string is a CEL expression, parsed at build so it can refer to the default prototype via `simulation`.
    pub fn set_end(&mut self, end: impl Into<Bound>) -> &mut Self {
        let end: Bound = end.into();
        self.end = Some(end.of_kind(BoundKind::End));
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

    pub fn channel(mut self, channel: impl Into<String>) -> Self {
        self.set_channel(channel);
        self
    }

    pub fn set_channel(&mut self, channel: impl Into<String>) -> &mut Self {
        self.channel = Some(channel.into());
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

    pub fn prototype(mut self, prototype: &Prototype) -> Self {
        self.set_prototype(prototype);
        self
    }

    /// Sets the default prototype, whose `start` and `end` expressions see as `simulation`; without it, or if it lacks either, expressions can't refer to `simulation`.
    pub fn set_prototype(&mut self, prototype: &Prototype) -> &mut Self {
        self.prototype = Some(prototype.clone());
        self
    }

    /// Resolves the prototype; errors if an expression doesn't parse or if `start` isn't before `end`.
    pub fn build(&self) -> Result<Prototype, Vec<BuildError>> {
        let now = self.now.unwrap_or_else(|| Utc::now().fixed_offset());
        let prototype = self.prototype.as_ref();
        let mut errors = vec![];
        let start = self
            .start
            .as_ref()
            .map(|bound| bound.resolve(now, prototype))
            .transpose()
            .map_err(|e| {
                errors.extend(
                    e.iter()
                        .map(|e| BuildError::simulation(SimulationKey::Start, e.message())),
                )
            });
        let end = self
            .end
            .as_ref()
            .map(|bound| bound.resolve(now, prototype))
            .transpose()
            .map_err(|e| {
                errors.extend(
                    e.iter()
                        .map(|e| BuildError::simulation(SimulationKey::Start, e.message())),
                )
            });
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
            return Err(vec![BuildError::simulation(SimulationKey::Start, message)]);
        }

        Ok(Prototype {
            name: self.name.clone(),
            start,
            end,
            limit: self.limit,
            channel: self.channel.clone(),
        })
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
            .set_end("prototype.end - days(1)");
        let now = now();
        let prototype = builder
            .set_now(now)
            .set_prototype(&Prototype {
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
