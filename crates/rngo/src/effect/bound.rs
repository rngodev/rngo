use crate::effect::prototype::Prototype;
use crate::error::ParseError;
use crate::moment::Moment;
use chrono::{DateTime, FixedOffset};

/// Which end of a window a [`Bound`] sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundKind {
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
pub enum BoundValue {
    Moment(Moment),
    Expression(String),
}

/// A start or end as declared. `kind` is `None` until a builder's `set_start` or `set_end` fills it in.
#[derive(Clone, Debug)]
pub struct Bound {
    pub kind: Option<BoundKind>,
    pub value: BoundValue,
}

impl From<Moment> for Bound {
    fn from(moment: Moment) -> Self {
        Bound {
            kind: None,
            value: BoundValue::Moment(moment),
        }
    }
}

impl From<DateTime<FixedOffset>> for Bound {
    fn from(timestamp: DateTime<FixedOffset>) -> Self {
        Bound {
            kind: None,
            value: BoundValue::Moment(timestamp.into()),
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

    /// Resolves the bound at `now`; `default`'s start and end, when both are set, are what an expression sees as `simulation`. Parse errors name the bound by its `kind`.
    pub(crate) fn resolve(
        &self,
        now: DateTime<FixedOffset>,
        default: Option<&Prototype>,
    ) -> Result<DateTime<FixedOffset>, Vec<ParseError>> {
        let moment = match &self.value {
            BoundValue::Moment(moment) => moment.clone(),
            BoundValue::Expression(expression) => {
                let simulation = default.and_then(|default| {
                    Some((Moment::from(default.start?), Moment::from(default.end?)))
                });
                let mut parser = Moment::parser();
                if let Some((start, end)) = &simulation {
                    parser = parser.simulation(start, end);
                }
                parser.parse(self.kind.map_or("bound", BoundKind::name), expression)?
            }
        };
        Ok(moment.resolve(now))
    }
}
