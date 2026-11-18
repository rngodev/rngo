use cel::extractors::This;
use cel::objects::Map;
use cel::{Context, ExecutionError, Program, Value};
use jiff::Timestamp;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) fn json_to_cel(value: serde_json::Value) -> Value {
    match value {
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => Value::Int(i),
            (_, Some(u), _) => Value::Int(u.min(i64::MAX as u64) as i64),
            (_, _, Some(f)) => Value::Float(f),
            _ => Value::Int(0),
        },
        serde_json::Value::String(s) => Value::String(s.into()),
        serde_json::Value::Array(a) => {
            Value::List(a.into_iter().map(json_to_cel).collect::<Vec<_>>().into())
        }
        serde_json::Value::Object(o) => Value::Map(Map::from(
            o.into_iter()
                .map(|(k, v)| (k, json_to_cel(v)))
                .collect::<HashMap<_, _>>(),
        )),
        serde_json::Value::Null => Value::Null,
    }
}

pub(crate) fn duration_value(seconds: i64) -> Value {
    cel::functions::duration(Arc::new(format!("{seconds}s")))
        .expect("whole seconds are a valid CEL duration")
}

/// Builds a CEL timestamp by evaluating `timestamp(..)` so no time crate has to be named here.
pub(crate) fn timestamp_value(timestamp: Timestamp) -> Value {
    let mut context = Context::default();
    context.add_variable_from_value("rfc3339", Value::String(timestamp.to_string().into()));
    Program::compile("timestamp(rfc3339)")
        .expect("static expression compiles")
        .execute(&context)
        .expect("a jiff timestamp is a valid CEL timestamp")
}

fn seconds(n: i64) -> Result<Value, ExecutionError> {
    Ok(duration_value(n))
}
fn minutes(n: i64) -> Result<Value, ExecutionError> {
    Ok(duration_value(n * 60))
}
fn hours(n: i64) -> Result<Value, ExecutionError> {
    Ok(duration_value(n * 3_600))
}
fn days(n: i64) -> Result<Value, ExecutionError> {
    Ok(duration_value(n * 86_400))
}
fn weeks(n: i64) -> Result<Value, ExecutionError> {
    Ok(duration_value(n * 604_800))
}
fn months(n: i64) -> Result<Value, ExecutionError> {
    Ok(duration_value(n * 2_419_200))
}
fn years(n: i64) -> Result<Value, ExecutionError> {
    Ok(duration_value(n * 31_536_000))
}

fn duration_seconds(function: &str, value: &Value) -> Result<i64, ExecutionError> {
    match value {
        Value::Duration(d) => Ok(d.num_seconds()),
        _ => Err(ExecutionError::function_error(
            function,
            "expected a duration",
        )),
    }
}

fn to_seconds(This(d): This<Value>) -> Result<f64, ExecutionError> {
    Ok(duration_seconds("toSeconds", &d)? as f64)
}
fn hz(n: i64, d: Value) -> Result<f64, ExecutionError> {
    Ok(n as f64 / duration_seconds("hz", &d)? as f64)
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

pub(crate) trait CelContextExt {
    fn with_time(&mut self) -> &mut Self;
    fn with_hertz(&mut self) -> &mut Self;
    fn with_strings(&mut self) -> &mut Self;
    fn with_now(&mut self, now: Timestamp) -> &mut Self;
    fn with_prototype(&mut self, start: Option<Timestamp>, end: Option<Timestamp>) -> &mut Self;
    fn with_offset(&mut self, offset: f64) -> &mut Self;
}

impl CelContextExt for Context<'static> {
    fn with_time(&mut self) -> &mut Self {
        self.add_function("seconds", seconds);
        let _ = self.add_variable("second", duration_value(1));

        self.add_function("minutes", minutes);
        let _ = self.add_variable("minute", duration_value(60));

        self.add_function("hours", hours);
        let _ = self.add_variable("hour", duration_value(3_600));

        self.add_function("days", days);
        let _ = self.add_variable("day", duration_value(86_400));

        self.add_function("weeks", weeks);
        let _ = self.add_variable("week", duration_value(604_800));

        self.add_function("months", months);
        let _ = self.add_variable("month", duration_value(2_419_200));

        self.add_function("years", years);
        let _ = self.add_variable("year", duration_value(31_536_000));

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

    fn with_now(&mut self, now: Timestamp) -> &mut Self {
        self.add_variable_from_value("now", timestamp_value(now));
        self
    }

    fn with_prototype(&mut self, start: Option<Timestamp>, end: Option<Timestamp>) -> &mut Self {
        let prototype: HashMap<String, Value> = [("start", start), ("end", end)]
            .into_iter()
            .filter_map(|(key, timestamp)| Some((key.to_string(), timestamp_value(timestamp?))))
            .collect();
        let _ = self.add_variable("prototype", Value::Map(Map::from(prototype)));
        self
    }

    fn with_offset(&mut self, offset: f64) -> &mut Self {
        let _ = self.add_variable("offset", offset);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cel::Program;

    fn eval(ctx: &Context<'static>, expr: &str) -> Value {
        Program::compile(expr).unwrap().execute(ctx).unwrap()
    }

    fn float(ctx: &Context<'static>, expr: &str) -> f64 {
        match eval(ctx, expr) {
            Value::Float(f) => f,
            v => panic!("expected float, got {:?}", v),
        }
    }

    fn duration(ctx: &Context<'static>, expr: &str) -> i64 {
        match eval(ctx, expr) {
            Value::Duration(d) => d.num_seconds(),
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

        assert_eq!(duration(&ctx, "second"), 1);
        assert_eq!(duration(&ctx, "minute"), 60);
        assert_eq!(duration(&ctx, "hour"), 3_600);
        assert_eq!(duration(&ctx, "day"), 86_400);
        assert_eq!(duration(&ctx, "week"), 604_800);
        assert_eq!(duration(&ctx, "month"), 2_419_200);
        assert_eq!(duration(&ctx, "year"), 31_536_000);
    }

    #[test]
    fn time_unit_functions() {
        let mut ctx = Context::default();
        ctx.with_time();

        assert_eq!(duration(&ctx, "seconds(90)"), 90);
        assert_eq!(duration(&ctx, "minutes(2)"), 120);
        assert_eq!(duration(&ctx, "hours(3)"), 10_800);
        assert_eq!(duration(&ctx, "days(1)"), 86_400);
        assert_eq!(duration(&ctx, "weeks(1)"), 604_800);
        assert_eq!(duration(&ctx, "months(1)"), 2_419_200);
        assert_eq!(duration(&ctx, "years(1)"), 31_536_000);
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
        let now = Timestamp::now();
        let mut ctx = Context::default();
        ctx.with_time().with_now(now);

        assert_eq!(eval(&ctx, "now"), timestamp_value(now));
    }

    #[test]
    fn now_arithmetic_with_durations() {
        let now = Timestamp::now();
        let mut ctx = Context::default();
        ctx.with_time().with_now(now);

        assert!(bool(&ctx, "now + day > now"));
        assert!(bool(&ctx, "now - hour < now"));
    }

    #[test]
    fn with_prototype_sets_map() {
        let start = "2024-01-01T00:00:00Z".parse::<Timestamp>().unwrap();
        let end = "2024-12-31T00:00:00Z".parse::<Timestamp>().unwrap();
        let mut ctx = Context::default();
        ctx.with_prototype(Some(start), Some(end));

        assert_eq!(eval(&ctx, "prototype.start"), timestamp_value(start));
        assert_eq!(eval(&ctx, "prototype.end"), timestamp_value(end));
    }

    #[test]
    fn with_prototype_only_sets_the_bounds_it_has() {
        let start = "2024-01-01T00:00:00Z".parse::<Timestamp>().unwrap();
        let mut ctx = Context::default();
        ctx.with_prototype(Some(start), None);

        assert_eq!(eval(&ctx, "prototype.start"), timestamp_value(start));
        assert!(
            Program::compile("prototype.end")
                .unwrap()
                .execute(&ctx)
                .is_err()
        );
    }

    #[test]
    fn with_offset_sets_variable() {
        let mut ctx = Context::default();
        ctx.with_offset(42.0);

        assert_eq!(eval(&ctx, "offset"), Value::Float(42.0));
    }

    #[test]
    fn with_offset_can_be_updated() {
        let mut ctx = Context::default();
        ctx.with_offset(10.0);
        ctx.with_offset(99.0);

        assert_eq!(eval(&ctx, "offset"), Value::Float(99.0));
    }

    #[test]
    fn now_timestamp_methods() {
        // 2024-03-15T10:30:45Z — a Friday, day 74 of year, month 2 (0-based)
        let now = "2024-03-15T10:30:45Z".parse::<Timestamp>().unwrap();
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
