use super::cel::CelContextExt;
use crate::spec::SpecError;
use cel::{Context, Program, Value};
use chrono::{DateTime, FixedOffset, NaiveDate, TimeDelta, TimeZone, Utc};

#[derive(Clone, Debug)]
pub enum Moment {
    Absolute(DateTime<FixedOffset>),
    Relative(TimeDelta),
}

impl Moment {
    pub fn parser<'a>() -> MomentParser<'a> {
        MomentParser {
            simulation_range: None,
        }
    }

    pub fn resolve(self, now: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
        match self {
            Moment::Absolute(date_time) => date_time,
            Moment::Relative(time_delta) => now + time_delta,
        }
    }
}

impl From<DateTime<FixedOffset>> for Moment {
    fn from(dt: DateTime<FixedOffset>) -> Self {
        Moment::Absolute(dt)
    }
}

impl From<TimeDelta> for Moment {
    fn from(delta: TimeDelta) -> Self {
        Moment::Relative(delta)
    }
}

pub struct MomentParser<'a> {
    simulation_range: Option<(&'a Moment, &'a Moment)>,
}

impl<'a> MomentParser<'a> {
    pub fn simulation(mut self, start: &'a Moment, end: &'a Moment) -> Self {
        self.simulation_range = Some((start, end));
        self
    }

    pub fn parse(&self, field_name: &str, expr: &str) -> Result<Moment, Vec<SpecError>> {
        if let Ok(dt) = DateTime::parse_from_rfc3339(expr) {
            return Ok(Moment::Absolute(dt));
        }

        if let Ok(date) = NaiveDate::parse_from_str(expr, "%Y-%m-%d") {
            let dt = Utc
                .from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
                .fixed_offset();
            return Ok(Moment::Absolute(dt));
        }

        let now = Utc::now().fixed_offset();
        let mut context = Context::default();
        context.with_time().with_now(now);

        if let Some((simulation_start, simulation_end)) = self.simulation_range {
            context.with_simulation(
                simulation_start.clone().resolve(now),
                simulation_end.clone().resolve(now),
            );
        };

        let program = Program::compile(expr).map_err(|e| {
            vec![SpecError {
                path: Some(vec![field_name.to_string()]),
                message: format!("could not compile {} expression: {}", field_name, e),
            }]
        })?;

        let value = program.execute(&context).map_err(|e| {
            vec![SpecError {
                path: Some(vec![field_name.to_string()]),
                message: format!("could not execute {} expression: {}", field_name, e),
            }]
        })?;

        let result_secs = match value {
            Value::Int(i) => i,
            Value::UInt(ui) => ui as i64,
            Value::Float(f) => f.round() as i64,
            Value::Timestamp(dt) => dt.timestamp(),
            Value::Duration(d) => (now + d).timestamp(),
            _ => {
                return Err(vec![SpecError {
                    path: Some(vec![field_name.to_string()]),
                    message: format!(
                        "{} expression must evaluate to a number or timestamp (seconds relative to now)",
                        field_name
                    ),
                }]);
            }
        };

        Ok(Moment::Relative(TimeDelta::seconds(
            result_secs - now.timestamp(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(expr: &str) -> Result<Moment, Vec<SpecError>> {
        Moment::parser().parse("field", expr)
    }

    #[test]
    fn parses_rfc3339() {
        let expected = DateTime::parse_from_rfc3339("2024-03-15T10:30:00Z").unwrap();
        let moment = parse("2024-03-15T10:30:00Z").unwrap();
        assert!(matches!(moment, Moment::Absolute(dt) if dt == expected));
    }

    #[test]
    fn parses_date_only() {
        let expected = DateTime::parse_from_rfc3339("2024-03-15T00:00:00Z").unwrap();
        let moment = parse("2024-03-15").unwrap();
        assert!(matches!(moment, Moment::Absolute(dt) if dt == expected));
    }

    #[test]
    fn parses_cel_now() {
        // `now` evaluates to the current timestamp, so the delta from now is 0
        let moment = parse("now").unwrap();
        assert!(matches!(moment, Moment::Relative(d) if d.num_seconds() == 0));
    }

    #[test]
    fn parses_cel_now_plus_duration() {
        let moment = parse("now + day").unwrap();
        assert!(matches!(moment, Moment::Relative(d) if d.num_seconds() == 86_400));
    }

    #[test]
    fn parses_cel_bare_duration() {
        // a bare duration is equivalent to now + that duration
        let moment = parse("day").unwrap();
        assert!(matches!(moment, Moment::Relative(d) if d.num_seconds() == 86_400));
    }

    #[test]
    fn parses_cel_now_minus_duration() {
        let moment = parse("now - weeks(2)").unwrap();
        assert!(matches!(moment, Moment::Relative(d) if d.num_seconds() == -2 * 604_800));
    }

    #[test]
    fn parses_cel_with_simulation_range() {
        let start_dt = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").unwrap();
        let end_dt = DateTime::parse_from_rfc3339("2024-12-31T00:00:00Z").unwrap();
        let start = Moment::Absolute(start_dt);
        let end = Moment::Absolute(end_dt);
        let parser = Moment::parser().simulation(&start, &end);

        // bare start/end resolve to their respective datetimes
        assert!(matches!(
            parser.parse("field", "simulation.start").unwrap(),
            Moment::Relative(_)
        ));
        assert!(matches!(
            parser.parse("field", "simulation.end").unwrap(),
            Moment::Relative(_)
        ));

        // arithmetic with durations works naturally since they are timestamps
        let moment = parser.parse("field", "simulation.end + days(20)").unwrap();
        assert!(
            matches!(moment, Moment::Relative(d) if d.num_seconds() == end_dt.timestamp() + 20 * 86_400 - Utc::now().fixed_offset().timestamp())
        );

        // timestamp methods work on simulation bounds
        assert!(matches!(
            parser
                .parse("field", "simulation.start.getFullYear()")
                .unwrap(),
            Moment::Relative(_)
        ));
        assert!(matches!(
            parser
                .parse("field", "simulation.end.getDayOfYear()")
                .unwrap(),
            Moment::Relative(_)
        ));
    }

    #[test]
    fn parses_cel_with_relative_simulation_range() {
        // start = now - 2 years, end = now + 5 hours
        let start = Moment::Relative(TimeDelta::seconds(-2 * 31_536_000));
        let end = Moment::Relative(TimeDelta::seconds(5 * 3_600));
        let parser = Moment::parser().simulation(&start, &end);

        // simulation.start + years(1) = (now - 2yr) + 1yr = now - 1yr
        let moment = parser.parse("field", "simulation.start + years(1)").unwrap();
        assert!(matches!(moment, Moment::Relative(d) if d.num_seconds() == -31_536_000));

        // simulation.end - hours(2) = (now + 5hr) - 2hr = now + 3hr
        let moment = parser.parse("field", "simulation.end - hours(2)").unwrap();
        assert!(matches!(moment, Moment::Relative(d) if d.num_seconds() == 3 * 3_600));
    }

    #[test]
    fn error_on_invalid_expression() {
        let errs = parse("???").unwrap_err();
        assert_eq!(errs[0].path, Some(vec!["field".to_string()]));
        assert!(errs[0].message.contains("field"));
    }

    #[test]
    fn error_on_wrong_type() {
        let errs = parse("true").unwrap_err();
        assert_eq!(errs[0].path, Some(vec!["field".to_string()]));
        assert!(errs[0].message.contains("field"));
    }
}
