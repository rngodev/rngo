use crate::cel::CelContextExt;
use crate::effect::prototype::Prototype;
use crate::error::{ParseError, ParseErrors};
use cel::{Context, Program, Value};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

/// Adds `delta` to `timestamp`, clamping to the earliest or latest representable instant instead of overflowing.
pub(crate) fn offset_timestamp(timestamp: Timestamp, delta: SignedDuration) -> Timestamp {
    timestamp
        .checked_add(delta)
        .unwrap_or(if delta.is_negative() {
            Timestamp::MIN
        } else {
            Timestamp::MAX
        })
}

#[derive(Clone, Debug)]
pub enum Moment {
    Absolute(Timestamp),
    Relative(SignedDuration),
}

impl Moment {
    pub fn parser<'a>() -> MomentParser<'a> {
        MomentParser { prototype: None }
    }

    pub fn resolve(self, now: Timestamp) -> Timestamp {
        match self {
            Moment::Absolute(date_time) => date_time,
            Moment::Relative(delta) => offset_timestamp(now, delta),
        }
    }
}

impl From<Timestamp> for Moment {
    fn from(dt: Timestamp) -> Self {
        Moment::Absolute(dt)
    }
}

impl From<SignedDuration> for Moment {
    fn from(delta: SignedDuration) -> Self {
        Moment::Relative(delta)
    }
}

pub struct MomentParser<'a> {
    prototype: Option<&'a Prototype>,
}

impl<'a> MomentParser<'a> {
    /// Lets expressions refer to `prototype.start` and `prototype.end`, whichever the prototype has.
    pub fn prototype(mut self, prototype: &'a Prototype) -> Self {
        self.prototype = Some(prototype);
        self
    }

    pub fn parse(&self, field_name: &str, expr: &str) -> Result<Moment, ParseErrors> {
        if let Ok(timestamp) = expr.parse::<Timestamp>() {
            return Ok(Moment::Absolute(timestamp));
        }

        if let Ok(date) = expr.parse::<Date>()
            && let Ok(zoned) = date.to_zoned(TimeZone::UTC)
        {
            return Ok(Moment::Absolute(zoned.timestamp()));
        }

        let now = Timestamp::now();
        let mut context = Context::default();
        context.with_time().with_now(now);

        if let Some(prototype) = self.prototype {
            context.with_prototype(prototype.start, prototype.end);
        };

        let program = Program::compile(expr).map_err(|e| {
            vec![
                ParseError::invalid(format!(
                    "could not compile {} expression: {}",
                    field_name, e
                ))
                .at([field_name.to_string()]),
            ]
        })?;

        let value = program.execute(&context).map_err(|e| {
            vec![
                ParseError::invalid(format!(
                    "could not execute {} expression: {}",
                    field_name, e
                ))
                .at([field_name.to_string()]),
            ]
        })?;

        let result_secs = match value {
            Value::Int(i) => i,
            Value::UInt(ui) => ui as i64,
            Value::Float(f) => f.round() as i64,
            Value::Timestamp(dt) => dt.timestamp(),
            Value::Duration(d) => now.as_second().saturating_add(d.num_seconds()),
            _ => {
                return Err(ParseError::invalid(format!(
                        "{} expression must evaluate to a number or timestamp (seconds relative to now)",
                        field_name
                    )).at([field_name.to_string()]).into());
            }
        };

        Ok(Moment::Relative(SignedDuration::from_secs(
            result_secs.saturating_sub(now.as_second()),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(expr: &str) -> Result<Moment, ParseErrors> {
        Moment::parser().parse("field", expr)
    }

    #[test]
    fn parses_rfc3339() {
        let expected = "2024-03-15T10:30:00Z".parse::<Timestamp>().unwrap();
        let moment = parse("2024-03-15T10:30:00Z").unwrap();
        assert!(matches!(moment, Moment::Absolute(dt) if dt == expected));
    }

    #[test]
    fn parses_date_only() {
        let expected = "2024-03-15T00:00:00Z".parse::<Timestamp>().unwrap();
        let moment = parse("2024-03-15").unwrap();
        assert!(matches!(moment, Moment::Absolute(dt) if dt == expected));
    }

    #[test]
    fn parses_cel_now() {
        // `now` evaluates to the current timestamp, so the delta from now is 0
        let moment = parse("now").unwrap();
        assert!(matches!(moment, Moment::Relative(d) if d.as_secs() == 0));
    }

    #[test]
    fn parses_cel_now_plus_duration() {
        let moment = parse("now + day").unwrap();
        assert!(matches!(moment, Moment::Relative(d) if d.as_secs() == 86_400));
    }

    #[test]
    fn parses_cel_bare_duration() {
        // a bare duration is equivalent to now + that duration
        let moment = parse("day").unwrap();
        assert!(matches!(moment, Moment::Relative(d) if d.as_secs() == 86_400));
    }

    #[test]
    fn parses_cel_now_minus_duration() {
        let moment = parse("now - weeks(2)").unwrap();
        assert!(matches!(moment, Moment::Relative(d) if d.as_secs() == -2 * 604_800));
    }

    #[test]
    fn parses_cel_with_prototype_range() {
        let start_dt = "2024-01-01T00:00:00Z".parse::<Timestamp>().unwrap();
        let end_dt = "2024-12-31T00:00:00Z".parse::<Timestamp>().unwrap();
        let prototype = Prototype {
            start: Some(start_dt),
            end: Some(end_dt),
            ..Prototype::default()
        };
        let parser = Moment::parser().prototype(&prototype);

        // bare start/end resolve to their respective datetimes
        assert!(matches!(
            parser.parse("field", "prototype.start").unwrap(),
            Moment::Relative(_)
        ));
        assert!(matches!(
            parser.parse("field", "prototype.end").unwrap(),
            Moment::Relative(_)
        ));

        // arithmetic with durations works naturally since they are timestamps
        let moment = parser.parse("field", "prototype.end + days(20)").unwrap();
        assert!(
            matches!(moment, Moment::Relative(d) if d.as_secs() == end_dt.as_second() + 20 * 86_400 - Timestamp::now().as_second())
        );

        // timestamp methods work on prototype bounds
        assert!(matches!(
            parser
                .parse("field", "prototype.start.getFullYear()")
                .unwrap(),
            Moment::Relative(_)
        ));
        assert!(matches!(
            parser
                .parse("field", "prototype.end.getDayOfYear()")
                .unwrap(),
            Moment::Relative(_)
        ));
    }

    #[test]
    fn a_prototype_with_only_a_start_exposes_only_its_start() {
        let start_dt = "2024-01-01T00:00:00Z".parse::<Timestamp>().unwrap();
        let prototype = Prototype {
            start: Some(start_dt),
            ..Prototype::default()
        };
        let parser = Moment::parser().prototype(&prototype);

        assert!(parser.parse("field", "prototype.start + days(1)").is_ok());
        assert!(parser.parse("field", "prototype.end").is_err());
    }

    #[test]
    fn error_on_invalid_expression() {
        let errs = parse("???").unwrap_err();
        assert_eq!(errs[0].path(), ["field"]);
        assert!(errs[0].message().contains("field"));
    }

    #[test]
    fn error_on_wrong_type() {
        let errs = parse("true").unwrap_err();
        assert_eq!(errs[0].path(), ["field"]);
        assert!(errs[0].message().contains("field"));
    }
}
