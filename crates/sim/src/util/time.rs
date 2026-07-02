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

    pub fn timestamp(&self, now: &DateTime<FixedOffset>) -> i64 {
        match self {
            Moment::Absolute(date_time) => date_time.timestamp(),
            Moment::Relative(time_delta) => (*now + *time_delta).timestamp(),
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
                simulation_start.timestamp(&now),
                simulation_end.timestamp(&now),
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
