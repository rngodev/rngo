use crate::effect::prototype::Prototype;
use crate::error::ParseErrors;
use crate::moment::Moment;
use jiff::{SignedDuration, Timestamp};

/// Which end of a window a [`Bound`] sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoundKind {
    Start,
    End,
}

impl BoundKind {
    fn name(self) -> &'static str {
        match self {
            BoundKind::Start => "start",
            BoundKind::End => "end",
        }
    }
}

/// What a [`Bound`] holds: a fixed moment, or a CEL expression resolved when the simulation builds.
#[derive(Clone, Debug)]
pub(crate) enum BoundValue {
    Moment(Moment),
    Expression(String),
}

/// A window's start or end as declared: a [`Moment`], a timestamp, a duration relative to now, or a CEL expression (from a `String` or `&str`, such as `"now - days(7)"`).
#[derive(Clone, Debug)]
pub struct Bound {
    pub(crate) kind: Option<BoundKind>,
    pub(crate) value: BoundValue,
}

impl From<Moment> for Bound {
    fn from(moment: Moment) -> Self {
        Bound {
            kind: None,
            value: BoundValue::Moment(moment),
        }
    }
}

impl From<Timestamp> for Bound {
    fn from(timestamp: Timestamp) -> Self {
        Bound {
            kind: None,
            value: BoundValue::Moment(timestamp.into()),
        }
    }
}

impl From<SignedDuration> for Bound {
    fn from(delta: SignedDuration) -> Self {
        Bound {
            kind: None,
            value: BoundValue::Moment(delta.into()),
        }
    }
}

impl From<String> for Bound {
    fn from(expression: String) -> Self {
        Bound {
            kind: None,
            value: BoundValue::Expression(expression),
        }
    }
}

impl From<&str> for Bound {
    fn from(expression: &str) -> Self {
        Bound {
            kind: None,
            value: BoundValue::Expression(expression.into()),
        }
    }
}

impl Bound {
    /// Returns the bound with its `kind` set.
    pub(crate) fn of_kind(mut self, kind: BoundKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Resolves the bound at `now`; `prototype`'s start and end, when both are set, are what an expression sees as `prototype.start` and `prototype.end`. Parse errors name the bound by its `kind`.
    pub(crate) fn resolve(
        &self,
        now: Timestamp,
        prototype: Option<&Prototype>,
    ) -> Result<Timestamp, ParseErrors> {
        let moment = match &self.value {
            BoundValue::Moment(moment) => moment.clone(),
            BoundValue::Expression(expression) => {
                let mut parser = Moment::parser();
                if let Some(prototype) = prototype {
                    parser = parser.prototype(prototype);
                }
                parser.parse(self.kind.map_or("bound", BoundKind::name), expression)?
            }
        };
        Ok(moment.resolve(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_is_relative_to_now() {
        let now: Timestamp = "2024-01-08T00:00:00Z".parse().unwrap();
        let bound = Bound::from(SignedDuration::from_hours(-24));

        assert_eq!(
            bound.resolve(now, None).unwrap(),
            now - SignedDuration::from_hours(24)
        );
    }
}
