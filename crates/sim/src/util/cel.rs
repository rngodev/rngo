use cel::objects::Map;
use cel::{Context, Value};
use std::collections::HashMap;

macro_rules! time_unit_fns {
    ($name:ident, $seconds:expr) => {
        pastey::paste! {
            fn [<$name _int_fn>](n: i64) -> f64     { (n * $seconds) as f64 }
            fn [<$name _int_int_fn>](n: i64) -> i64  { n * $seconds }
            fn [<$name _float_fn>](n: f64) -> f64    { n * ($seconds as f64) }
            fn [<$name _float_int_fn>](n: f64) -> i64 { (n * ($seconds as f64)) as i64 }
        }
    };
}

time_unit_fns!(seconds, 1_i64);
time_unit_fns!(minutes, 60_i64);
time_unit_fns!(hours, 3_600_i64);
time_unit_fns!(days, 86_400_i64);
time_unit_fns!(weeks, 604_800_i64);
time_unit_fns!(months, 2_419_200_i64); // 28 days
time_unit_fns!(years, 31_536_000_i64); // 365 days

macro_rules! register_time_fns {
    ($ctx:expr, $plural:literal, $singular:literal, $prefix:ident) => {
        pastey::paste! {
            $ctx.add_function($plural, [<$prefix _int_fn>]);
            $ctx.add_function($plural, [<$prefix _int_int_fn>]);
            $ctx.add_function($plural, [<$prefix _float_fn>]);
            $ctx.add_function($plural, [<$prefix _float_int_fn>]);
            let _ = $ctx.add_variable($singular, [<$prefix _int_fn>](1));
        }
    };
}

pub struct CelContextBuilder {
    empty: bool,
    time: bool,
    offset: Option<i64>,
    simulation_start: Option<i64>,
    simulation_end: Option<i64>,
}

impl CelContextBuilder {
    pub fn default() -> Self {
        Self {
            empty: false,
            time: false,
            offset: None,
            simulation_start: None,
            simulation_end: None,
        }
    }

    pub fn time(&mut self) -> &mut Self {
        self.time = true;
        self
    }

    pub fn offset(&mut self, offset: i64) -> &mut Self {
        self.offset = Some(offset);
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
            register_time_fns!(context, "seconds", "second", seconds);
            register_time_fns!(context, "minutes", "minute", minutes);
            register_time_fns!(context, "hours", "hour", hours);
            register_time_fns!(context, "days", "day", days);
            register_time_fns!(context, "weeks", "week", weeks);
            register_time_fns!(context, "months", "month", months);
            register_time_fns!(context, "years", "year", years);
        }

        if let Some(offset) = self.offset {
            let _ = context.add_variable("offset", offset);
            let _ = context.add_variable("now", offset);
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
