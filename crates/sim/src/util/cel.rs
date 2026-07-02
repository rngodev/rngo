use cel::extractors::This;
use cel::objects::Map;
use cel::{Context, Value};
use chrono::{DateTime, Duration, FixedOffset};
use std::collections::HashMap;

fn seconds(n: i64) -> Duration { Duration::seconds(n) }
fn minutes(n: i64) -> Duration { Duration::seconds(n * 60) }
fn hours(n: i64) -> Duration   { Duration::seconds(n * 3_600) }
fn days(n: i64) -> Duration    { Duration::seconds(n * 86_400) }
fn weeks(n: i64) -> Duration   { Duration::seconds(n * 604_800) }
fn months(n: i64) -> Duration  { Duration::seconds(n * 2_419_200) } // 28 days
fn years(n: i64) -> Duration   { Duration::seconds(n * 31_536_000) } // 365 days

fn to_seconds(This(d): This<Duration>) -> f64 { d.num_seconds() as f64 }

pub trait CelContextExt {
    fn with_time(&mut self) -> &mut Self;
    fn with_now(&mut self, now: DateTime<FixedOffset>) -> &mut Self;
    fn with_simulation(&mut self, start: i64, end: i64) -> &mut Self;
    fn with_offset(&mut self, offset: i64) -> &mut Self;
}

impl CelContextExt for Context<'static> {
    fn with_time(&mut self) -> &mut Self {
        self.add_function("seconds", seconds);
        let _ = self.add_variable("second", Value::Duration(Duration::seconds(1)));

        self.add_function("minutes", minutes);
        let _ = self.add_variable("minute", Value::Duration(Duration::seconds(60)));

        self.add_function("hours", hours);
        let _ = self.add_variable("hour", Value::Duration(Duration::seconds(3_600)));

        self.add_function("days", days);
        let _ = self.add_variable("day", Value::Duration(Duration::seconds(86_400)));

        self.add_function("weeks", weeks);
        let _ = self.add_variable("week", Value::Duration(Duration::seconds(604_800)));

        self.add_function("months", months);
        let _ = self.add_variable("month", Value::Duration(Duration::seconds(2_419_200)));

        self.add_function("years", years);
        let _ = self.add_variable("year", Value::Duration(Duration::seconds(31_536_000)));

        self.add_function("toSeconds", to_seconds);

        self
    }

    fn with_now(&mut self, now: DateTime<FixedOffset>) -> &mut Self {
        self.add_variable_from_value("now", Value::Timestamp(now));
        self
    }

    fn with_simulation(&mut self, start: i64, end: i64) -> &mut Self {
        let sim_map: HashMap<String, Value> = [
            ("start".to_string(), Value::Int(start)),
            ("end".to_string(), Value::Int(end)),
        ]
        .into();
        let _ = self.add_variable("simulation", Value::Map(Map::from(sim_map)));
        self
    }

    fn with_offset(&mut self, offset: i64) -> &mut Self {
        let _ = self.add_variable("offset", offset);
        self
    }
}
