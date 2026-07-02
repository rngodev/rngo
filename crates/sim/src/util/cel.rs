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

pub struct CelContextBuilder {
    empty: bool,
    time: bool,
    now: Option<DateTime<FixedOffset>>,
    simulation_start: Option<i64>,
    simulation_end: Option<i64>,
}

impl CelContextBuilder {
    pub fn default() -> Self {
        Self {
            empty: false,
            time: false,
            now: None,
            simulation_start: None,
            simulation_end: None,
        }
    }

    pub fn time(&mut self) -> &mut Self {
        self.time = true;
        self
    }

    pub fn set_now(&mut self, now: DateTime<FixedOffset>) -> &mut Self {
        self.now = Some(now);
        self
    }

    pub fn simulation(&mut self, start: i64, end: i64) -> &mut Self {
        self.simulation_start = Some(start);
        self.simulation_end = Some(end);
        self
    }

    pub fn build(self) -> Context<'static> {
        let mut context = if self.empty {
            Context::empty()
        } else {
            Context::default()
        };

        if self.time {
            context.add_function("seconds", seconds);
            let _ = context.add_variable("second", Value::Duration(Duration::seconds(1)));

            context.add_function("minutes", minutes);
            let _ = context.add_variable("minute", Value::Duration(Duration::seconds(60)));

            context.add_function("hours", hours);
            let _ = context.add_variable("hour", Value::Duration(Duration::seconds(3_600)));

            context.add_function("days", days);
            let _ = context.add_variable("day", Value::Duration(Duration::seconds(86_400)));

            context.add_function("weeks", weeks);
            let _ = context.add_variable("week", Value::Duration(Duration::seconds(604_800)));

            context.add_function("months", months);
            let _ = context.add_variable("month", Value::Duration(Duration::seconds(2_419_200)));

            context.add_function("years", years);
            let _ = context.add_variable("year", Value::Duration(Duration::seconds(31_536_000)));

            context.add_function("toSeconds", to_seconds);
        }

        if let Some(now) = self.now {
            context.add_variable_from_value("now", Value::Timestamp(now));
        }

        if self.simulation_start.is_some() || self.simulation_end.is_some() {
            let mut sim_map: HashMap<String, Value> = HashMap::new();

            if let Some(simulation_start) = self.simulation_start {
                sim_map.insert("start".to_string(), Value::Int(simulation_start));
            }

            if let Some(simulation_end) = self.simulation_end {
                sim_map.insert("end".to_string(), Value::Int(simulation_end));
            }

            let _ = context.add_variable("simulation", Value::Map(Map::from(sim_map)));
        }

        context
    }
}
