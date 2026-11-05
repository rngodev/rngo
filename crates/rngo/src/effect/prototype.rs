use crate::build::BuildError;
use crate::moment::Moment;
use chrono::{DateTime, FixedOffset};
use std::num::NonZeroU64;

/// A start or end as declared: a fixed moment, or a CEL expression resolved when the simulation builds.
#[derive(Clone, Debug)]
pub enum Bound {
    Moment(Moment),
    Expression(String),
}

/// A time window and an input cap as declared, each optional; an effect takes the parts it doesn't set itself from its prototype.
#[derive(Clone, Debug, Default)]
pub struct Prototype {
    pub start: Option<Bound>,
    pub end: Option<Bound>,
    pub limit: Option<NonZeroU64>,
}

impl Prototype {
    pub fn start(mut self, start: Moment) -> Self {
        self.set_start(start);
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.start = Some(Bound::Moment(start));
        self
    }

    /// Sets `start` from an expression, which can refer to the default prototype via `simulation`.
    pub fn set_start_expression(&mut self, expression: String) -> &mut Self {
        self.start = Some(Bound::Expression(expression));
        self
    }

    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.end = Some(Bound::Moment(end));
        self
    }

    /// Sets `end` from an expression, which can refer to the default prototype via `simulation`.
    pub fn set_end_expression(&mut self, expression: String) -> &mut Self {
        self.end = Some(Bound::Expression(expression));
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

    /// Resolves the bounds at `now`; errors if an expression doesn't parse, with each error built by `error` from the field name and message.
    pub fn resolve(
        &self,
        now: DateTime<FixedOffset>,
        default: Option<&ResolvedPrototype>,
        error: impl Fn(&str, String) -> BuildError,
    ) -> Result<ResolvedPrototype, Vec<BuildError>> {
        let mut errors = vec![];
        let start = resolve_bound("start", self.start.as_ref(), now, default, &error)
            .map_err(|e| errors.extend(e));
        let end = resolve_bound("end", self.end.as_ref(), now, default, &error)
            .map_err(|e| errors.extend(e));
        match (start, end) {
            (Ok(start), Ok(end)) => Ok(ResolvedPrototype {
                start,
                end,
                limit: self.limit,
            }),
            _ => Err(errors),
        }
    }
}

/// A [`Prototype`] with its bounds resolved to instants; whatever was unset stays unset.
#[derive(Clone, Debug, Default)]
pub struct ResolvedPrototype {
    pub start: Option<DateTime<FixedOffset>>,
    pub end: Option<DateTime<FixedOffset>>,
    pub limit: Option<NonZeroU64>,
}

/// Resolves one declared bound at `now`; `default`'s start and end, when both are set, are what expressions see as `simulation`.
pub(crate) fn resolve_bound(
    field: &str,
    bound: Option<&Bound>,
    now: DateTime<FixedOffset>,
    default: Option<&ResolvedPrototype>,
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
    use super::{Prototype, ResolvedPrototype};
    use crate::build::{BuildError, SimulationKey};
    use crate::moment::Moment;
    use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
    use std::num::NonZeroU64;

    fn now() -> DateTime<FixedOffset> {
        let now = Utc::now().fixed_offset();
        now - TimeDelta::nanoseconds(now.timestamp_subsec_nanos().into())
    }

    fn error(_field: &str, message: String) -> BuildError {
        BuildError::simulation(SimulationKey::Start, message)
    }

    #[test]
    fn leaves_unset_parts_unset() {
        let prototype = Prototype::default().resolve(now(), None, error).unwrap();
        assert!(prototype.start.is_none() && prototype.end.is_none() && prototype.limit.is_none());
    }

    #[test]
    fn resolves_moments_expressions_and_limit() {
        let mut declared = Prototype::default().limit(NonZeroU64::new(3).unwrap());
        declared
            .set_start(Moment::Relative(TimeDelta::days(-2)))
            .set_end_expression("simulation.end - days(1)".into());
        let now = now();
        let default = ResolvedPrototype {
            start: Some(now - TimeDelta::days(30)),
            end: Some(now),
            limit: None,
        };
        let prototype = declared.resolve(now, Some(&default), error).unwrap();

        assert_eq!(prototype.start, Some(now - TimeDelta::days(2)));
        assert_eq!(prototype.end, Some(now - TimeDelta::days(1)));
        assert_eq!(prototype.limit, NonZeroU64::new(3));
    }

    #[test]
    fn an_expression_that_does_not_parse_is_an_error() {
        let mut declared = Prototype::default();
        declared.set_start_expression("nonsense(".into());
        assert!(declared.resolve(now(), None, error).is_err());
    }
}
