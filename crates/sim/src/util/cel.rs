use cel::extractors::This;
use cel::objects::Map;
use cel::{Context, Value};
use chrono::{DateTime, Duration, FixedOffset};
use std::collections::HashMap;
use std::sync::Arc;

fn seconds(n: i64) -> Duration {
    Duration::seconds(n)
}
fn minutes(n: i64) -> Duration {
    Duration::seconds(n * 60)
}
fn hours(n: i64) -> Duration {
    Duration::seconds(n * 3_600)
}
fn days(n: i64) -> Duration {
    Duration::seconds(n * 86_400)
}
fn weeks(n: i64) -> Duration {
    Duration::seconds(n * 604_800)
}
fn months(n: i64) -> Duration {
    Duration::seconds(n * 2_419_200)
} // 28 days
fn years(n: i64) -> Duration {
    Duration::seconds(n * 31_536_000)
} // 365 days

fn to_seconds(This(d): This<Duration>) -> f64 {
    d.num_seconds() as f64
}
fn hz(n: i64, d: Duration) -> f64 {
    n as f64 / d.num_seconds() as f64
}

fn join(This(list): This<Arc<Vec<Value>>>, separator: Arc<String>) -> String {
    list.iter()
        .map(|v| match v {
            Value::String(s) => s.to_string(),
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join(separator.as_str())
}

pub trait CelContextExt {
    fn with_time(&mut self) -> &mut Self;
    fn with_hertz(&mut self) -> &mut Self;
    fn with_strings(&mut self) -> &mut Self;
    fn with_now(&mut self, now: DateTime<FixedOffset>) -> &mut Self;
    fn with_simulation(
        &mut self,
        start: DateTime<FixedOffset>,
        end: DateTime<FixedOffset>,
    ) -> &mut Self;
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

    fn with_hertz(&mut self) -> &mut Self {
        self.add_function("hz", hz);
        self
    }

    fn with_strings(&mut self) -> &mut Self {
        self.add_function("join", join);
        self
    }

    fn with_now(&mut self, now: DateTime<FixedOffset>) -> &mut Self {
        self.add_variable_from_value("now", Value::Timestamp(now));
        self
    }

    fn with_simulation(
        &mut self,
        start: DateTime<FixedOffset>,
        end: DateTime<FixedOffset>,
    ) -> &mut Self {
        let sim_map: HashMap<String, Value> = [
            ("start".to_string(), Value::Timestamp(start)),
            ("end".to_string(), Value::Timestamp(end)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use cel::Program;
    use chrono::Utc;

    fn eval(ctx: &Context<'static>, expr: &str) -> Value {
        Program::compile(expr).unwrap().execute(ctx).unwrap()
    }

    fn float(ctx: &Context<'static>, expr: &str) -> f64 {
        match eval(ctx, expr) {
            Value::Float(f) => f,
            v => panic!("expected float, got {:?}", v),
        }
    }

    fn duration(ctx: &Context<'static>, expr: &str) -> Duration {
        match eval(ctx, expr) {
            Value::Duration(d) => d,
            v => panic!("expected duration, got {:?}", v),
        }
    }

    fn string(ctx: &Context<'static>, expr: &str) -> String {
        match eval(ctx, expr) {
            Value::String(s) => (*s).clone(),
            v => panic!("expected string, got {:?}", v),
        }
    }

    fn bool(ctx: &Context<'static>, expr: &str) -> std::primitive::bool {
        match eval(ctx, expr) {
            Value::Bool(b) => b,
            v => panic!("expected bool, got {:?}", v),
        }
    }

    #[test]
    fn time_unit_variables() {
        let mut ctx = Context::default();
        ctx.with_time();

        assert_eq!(duration(&ctx, "second"), Duration::seconds(1));
        assert_eq!(duration(&ctx, "minute"), Duration::seconds(60));
        assert_eq!(duration(&ctx, "hour"), Duration::seconds(3_600));
        assert_eq!(duration(&ctx, "day"), Duration::seconds(86_400));
        assert_eq!(duration(&ctx, "week"), Duration::seconds(604_800));
        assert_eq!(duration(&ctx, "month"), Duration::seconds(2_419_200));
        assert_eq!(duration(&ctx, "year"), Duration::seconds(31_536_000));
    }

    #[test]
    fn time_unit_functions() {
        let mut ctx = Context::default();
        ctx.with_time();

        assert_eq!(duration(&ctx, "seconds(90)"), Duration::seconds(90));
        assert_eq!(duration(&ctx, "minutes(2)"), Duration::seconds(120));
        assert_eq!(duration(&ctx, "hours(3)"), Duration::seconds(10_800));
        assert_eq!(duration(&ctx, "days(1)"), Duration::seconds(86_400));
        assert_eq!(duration(&ctx, "weeks(1)"), Duration::seconds(604_800));
        assert_eq!(duration(&ctx, "months(1)"), Duration::seconds(2_419_200));
        assert_eq!(duration(&ctx, "years(1)"), Duration::seconds(31_536_000));
    }

    #[test]
    fn to_seconds_method() {
        let mut ctx = Context::default();
        ctx.with_time();

        assert_eq!(float(&ctx, "day.toSeconds()"), 86_400.0);
        assert_eq!(float(&ctx, "hours(2).toSeconds()"), 7_200.0);
        assert_eq!(float(&ctx, "minute.toSeconds()"), 60.0);
    }

    #[test]
    fn hz_function() {
        let mut ctx = Context::default();
        ctx.with_time().with_hertz();

        assert_eq!(float(&ctx, "hz(1, day)"), 1.0 / 86_400.0);
        assert_eq!(float(&ctx, "hz(3, hour)"), 3.0 / 3_600.0);
        assert_eq!(float(&ctx, "hz(2, week)"), 2.0 / 604_800.0);
    }

    #[test]
    fn hz_equals_manual_division() {
        let mut ctx = Context::default();
        ctx.with_time().with_hertz();

        assert_eq!(
            float(&ctx, "hz(5, day)"),
            float(&ctx, "5.0 / day.toSeconds()")
        );
    }

    #[test]
    fn with_now_sets_timestamp() {
        let now = Utc::now().fixed_offset();
        let mut ctx = Context::default();
        ctx.with_time().with_now(now);

        assert_eq!(eval(&ctx, "now"), Value::Timestamp(now));
    }

    #[test]
    fn now_arithmetic_with_durations() {
        let now = Utc::now().fixed_offset();
        let mut ctx = Context::default();
        ctx.with_time().with_now(now);

        assert!(bool(&ctx, "now + day > now"));
        assert!(bool(&ctx, "now - hour < now"));
    }

    #[test]
    fn with_simulation_sets_map() {
        let start = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").unwrap();
        let end = DateTime::parse_from_rfc3339("2024-12-31T00:00:00Z").unwrap();
        let mut ctx = Context::default();
        ctx.with_simulation(start, end);

        assert_eq!(eval(&ctx, "simulation.start"), Value::Timestamp(start));
        assert_eq!(eval(&ctx, "simulation.end"), Value::Timestamp(end));
    }

    #[test]
    fn with_offset_sets_variable() {
        let mut ctx = Context::default();
        ctx.with_offset(42);

        assert_eq!(eval(&ctx, "offset"), Value::Int(42));
    }

    #[test]
    fn with_offset_can_be_updated() {
        let mut ctx = Context::default();
        ctx.with_offset(10);
        ctx.with_offset(99);

        assert_eq!(eval(&ctx, "offset"), Value::Int(99));
    }

    #[test]
    fn now_timestamp_methods() {
        // 2024-03-15T10:30:45Z — a Friday, day 74 of year, month 2 (0-based)
        let now = DateTime::parse_from_rfc3339("2024-03-15T10:30:45Z").unwrap();
        let mut ctx = Context::default();
        ctx.with_now(now);

        assert_eq!(eval(&ctx, "now.getFullYear()"), Value::Int(2024));
        assert_eq!(eval(&ctx, "now.getMonth()"), Value::Int(2)); // 0-based
        assert_eq!(eval(&ctx, "now.getDayOfYear()"), Value::Int(74)); // 0-based
        assert_eq!(eval(&ctx, "now.getDayOfMonth()"), Value::Int(14)); // 0-based
        assert_eq!(eval(&ctx, "now.getDate()"), Value::Int(15)); // 1-based
        assert_eq!(eval(&ctx, "now.getDayOfWeek()"), Value::Int(5)); // 0=Sun
        assert_eq!(eval(&ctx, "now.getHours()"), Value::Int(10));
        assert_eq!(eval(&ctx, "now.getMinutes()"), Value::Int(30));
        assert_eq!(eval(&ctx, "now.getSeconds()"), Value::Int(45));
    }

    #[test]
    fn join_method_joins_list_of_strings() {
        let mut ctx = Context::default();
        ctx.with_strings();

        assert_eq!(
            string(&ctx, "['a', 'b', 'c'].join('-')"),
            "a-b-c".to_string()
        );
        assert_eq!(string(&ctx, "['solo'].join(', ')"), "solo".to_string());
        assert_eq!(string(&ctx, "[].join(', ')"), "".to_string());
    }

    #[test]
    fn unit_aliases_match_functions() {
        let mut ctx = Context::default();
        ctx.with_time();

        assert!(bool(&ctx, "second == seconds(1)"));
        assert!(bool(&ctx, "minute == minutes(1)"));
        assert!(bool(&ctx, "hour == hours(1)"));
        assert!(bool(&ctx, "day == days(1)"));
        assert!(bool(&ctx, "week == weeks(1)"));
        assert!(bool(&ctx, "month == months(1)"));
        assert!(bool(&ctx, "year == years(1)"));
    }
}
