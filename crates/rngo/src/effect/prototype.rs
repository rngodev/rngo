use crate::effect::bound::{Bound, BoundKind};
use crate::error::{BuildError, BuildErrors, SimulationKey};
use jiff::Timestamp;
use std::num::NonZeroU64;

/// A time window and an input cap, each optional; an effect takes the parts it doesn't set itself from its prototype.
#[derive(Clone, Debug, Default)]
pub(crate) struct Prototype {
    pub(crate) start: Option<Timestamp>,
    pub(crate) end: Option<Timestamp>,
    pub(crate) limit: Option<NonZeroU64>,
    /// The key of the channel effects on this prototype write to unless they set their own.
    pub(crate) channel: Option<String>,
}

impl Prototype {
    #[cfg(test)]
    pub(crate) fn builder() -> PrototypeBuilder {
        PrototypeBuilder::default()
    }
}

/// Describes a prototype: defaults for the effects that name it; whatever is left unset stays unset.
#[derive(Clone, Debug, Default)]
pub struct PrototypeBuilder {
    name: Option<String>,
    start: Option<Bound>,
    end: Option<Bound>,
    limit: Option<NonZeroU64>,
    channel: Option<String>,
    now: Option<Timestamp>,
    prototype: Option<Prototype>,
}

impl PrototypeBuilder {
    pub(crate) fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// When effects on this prototype start firing, unless they set their own start.
    pub fn start(mut self, start: impl Into<Bound>) -> Self {
        let start: Bound = start.into();
        self.start = Some(start.of_kind(BoundKind::Start));
        self
    }

    /// When effects on this prototype stop firing, unless they set their own end.
    pub fn end(mut self, end: impl Into<Bound>) -> Self {
        let end: Bound = end.into();
        self.end = Some(end.of_kind(BoundKind::End));
        self
    }

    /// The most inputs each effect on this prototype produces, unless it sets its own limit.
    pub fn limit(mut self, limit: NonZeroU64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// The key of the channel effects on this prototype write to, unless they set their own.
    pub fn channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    pub(crate) fn now(mut self, now: Timestamp) -> Self {
        self.now = Some(now);
        self
    }

    pub(crate) fn prototype(mut self, prototype: &Prototype) -> Self {
        self.prototype = Some(prototype.clone());
        self
    }

    /// Resolves the prototype; errors if an expression doesn't parse or if `start` isn't before `end`.
    pub(crate) fn build(&self) -> Result<Prototype, BuildErrors> {
        let now = self.now.unwrap_or_else(Timestamp::now);
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
            return Err(errors.into());
        };

        if let (Some(start), Some(end)) = (start, end)
            && start >= end
        {
            let message = match &self.name {
                Some(name) => format!("prototype \"{name}\": start must be before end"),
                None => "start must be before end".into(),
            };
            return Err(BuildError::simulation(SimulationKey::Start, message).into());
        }

        Ok(Prototype {
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
    use jiff::{SignedDuration, Timestamp};
    use std::num::NonZeroU64;

    fn now() -> Timestamp {
        let now = Timestamp::now();
        Timestamp::from_second(now.as_second()).unwrap()
    }

    #[test]
    fn leaves_unset_parts_unset() {
        let prototype = Prototype::builder().build().unwrap();
        assert!(prototype.start.is_none() && prototype.end.is_none() && prototype.limit.is_none());
    }

    #[test]
    fn resolves_moments_expressions_and_limit() {
        let now = now();
        let prototype = Prototype::builder()
            .limit(NonZeroU64::new(3).unwrap())
            .start(Moment::Relative(SignedDuration::from_hours(24 * -2)))
            .end("prototype.end - days(1)")
            .now(now)
            .prototype(&Prototype {
                start: Some(now - SignedDuration::from_hours(24 * 30)),
                end: Some(now),
                ..Prototype::default()
            })
            .build()
            .unwrap();

        assert_eq!(
            prototype.start,
            Some(now - SignedDuration::from_hours(24 * 2))
        );
        assert_eq!(prototype.end, Some(now - SignedDuration::from_hours(24)));
        assert_eq!(prototype.limit, NonZeroU64::new(3));
    }

    #[test]
    fn a_prototype_ending_before_it_starts_is_an_error() {
        let errors = Prototype::builder()
            .name("backwards")
            .start(Moment::Relative(SignedDuration::from_hours(-24)))
            .end(Moment::Relative(SignedDuration::from_hours(24 * -60)))
            .build()
            .unwrap_err();
        assert!(errors[0].message().contains("backwards"));
    }
}
