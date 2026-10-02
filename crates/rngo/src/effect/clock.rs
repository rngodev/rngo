use crate::BuildError;
use crate::cel::CelContextExt;
use cel::{Context, Program, Value};
use rand::RngExt;
use rand_pcg::Pcg32;
use rand_seeder::Seeder;
use std::cell::RefCell;

/// Poisson event clock yielding offsets in milliseconds since simulation start.
#[derive(Debug)]
pub struct Clock {
    rng: Pcg32,
    rate_function: RateFunction,
    last: i64,
}

impl Clock {
    pub fn builder() -> ClockBuilder {
        ClockBuilder::new()
    }
}

impl Iterator for Clock {
    type Item = i64;

    fn next(&mut self) -> Option<Self::Item> {
        let u: f64 = self.rng.random();
        let rate = self.rate_function.offset_rate(self.last as f64 / 1000.0);
        let interval = -u.ln() / rate;
        self.last += (interval * 1000.0).round() as i64;
        Some(self.last)
    }
}

pub struct ClockBuilder {
    key: String,
    seed: u64,
    rate: ClockRate,
    start_offset: i64,
}

enum ClockRate {
    Hertz(f64),
    Expression(String),
}

impl ClockBuilder {
    pub fn new() -> Self {
        ClockBuilder {
            key: String::new(),
            seed: 1,
            rate: ClockRate::Expression("hz(1, day)".into()),
            start_offset: 0,
        }
    }

    pub fn key(mut self, key: String) -> Self {
        self.key = key;
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn hertz(mut self, hertz: f64) -> Self {
        self.rate = ClockRate::Hertz(hertz);
        self
    }

    pub fn expression(mut self, expression: String) -> Self {
        self.rate = ClockRate::Expression(expression);
        self
    }

    /// Sets the clock's starting offset in milliseconds since simulation start.
    pub fn start_offset(mut self, offset: i64) -> Self {
        self.start_offset = offset;
        self
    }

    pub fn build(self) -> Result<Clock, Vec<BuildError>> {
        let rng: Pcg32 = Seeder::from(&format!("{}-{}", self.seed, self.key)).into_rng();

        let rate_function = match self.rate {
            ClockRate::Hertz(hertz) => RateFunction::Fixed(hertz),
            ClockRate::Expression(expression) => {
                let program = Program::compile(&expression).map_err(|e| {
                    vec![BuildError::Effect {
                        effect: self.key.clone(),
                        key: crate::EffectKey::Trigger,
                        message: format!(
                            "could not compile expression: {}",
                            e.errors
                                .into_iter()
                                .map(|e| e.msg)
                                .collect::<Vec<_>>()
                                .join(",")
                        ),
                    }]
                })?;

                let mut context = Context::default();
                context.with_time().with_hertz();
                let references = program.references();

                if references.variables().contains(&"offset") {
                    let _ = context.add_variable("offset", 0.0);

                    program.execute(&context).map_err(|e| {
                        vec![BuildError::Effect {
                            effect: self.key.clone(),
                            key: crate::EffectKey::Trigger,
                            message: format!("could not execute expression: {e}"),
                        }]
                    })?;

                    RateFunction::Dynamic {
                        expression,
                        cache: RefCell::new(Some((program, context))),
                    }
                } else {
                    let value = program.execute(&context).map_err(|e| {
                        vec![BuildError::Effect {
                            effect: self.key.clone(),
                            key: crate::EffectKey::Trigger,
                            message: format!("could not execute expression: {e}"),
                        }]
                    })?;

                    let static_rate = match value {
                        Value::Int(i) => Some(i as f64),
                        Value::UInt(ui) => Some(ui as f64),
                        Value::Float(f) => Some(f),
                        _ => None,
                    };

                    match static_rate {
                        Some(rate) => RateFunction::Fixed(rate),
                        None => {
                            return Err(vec![BuildError::Effect {
                                effect: self.key,
                                key: crate::EffectKey::Trigger,
                                message: format!("rate expression non-numeric: {expression}"),
                            }]);
                        }
                    }
                }
            }
        };

        Ok(Clock {
            rng,
            rate_function,
            last: self.start_offset,
        })
    }
}

pub enum RateFunction {
    Fixed(f64),
    Dynamic {
        expression: String,
        cache: RefCell<Option<(Program, Context<'static>)>>,
    },
}

impl Clone for RateFunction {
    fn clone(&self) -> Self {
        match self {
            RateFunction::Fixed(rate) => RateFunction::Fixed(*rate),
            RateFunction::Dynamic { expression, .. } => RateFunction::Dynamic {
                expression: expression.clone(),
                cache: RefCell::new(None),
            },
        }
    }
}

impl std::fmt::Debug for RateFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RateFunction::Fixed(rate) => f.debug_tuple("Static").field(rate).finish(),
            RateFunction::Dynamic { expression, .. } => f
                .debug_struct("Dynamic")
                .field("expression", expression)
                .finish(),
        }
    }
}

impl RateFunction {
    fn offset_rate(&self, offset: f64) -> f64 {
        match self {
            RateFunction::Fixed(rate) => *rate,
            RateFunction::Dynamic { expression, cache } => {
                let mut cache_ref = cache.borrow_mut();

                // Lazily initialize program and context after deserialization
                if cache_ref.is_none() {
                    let program = Program::compile(expression).unwrap();
                    let mut context = Context::default();
                    context.with_time();
                    *cache_ref = Some((program, context));
                }

                let (prog, ctx) = cache_ref.as_mut().unwrap();
                ctx.with_offset(offset);
                match prog.execute(ctx) {
                    Ok(value) => match value {
                        Value::Float(num) => num,
                        Value::Int(num) => num as f64,
                        Value::UInt(num) => num as f64,
                        v => panic!("Unexpected rate function result: {:?}", v),
                    },
                    Err(e) => panic!("Error executing rate function: {:?}", e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events_in_first_seconds(hertz: f64, seconds: i64) -> usize {
        let clock = Clock::builder()
            .key("c".into())
            .hertz(hertz)
            .build()
            .unwrap();
        clock.take_while(|&offset| offset <= seconds * 1000).count()
    }

    #[test]
    fn fixed_clock_yields_requested_rate_above_one_hertz() {
        let seconds = 1000;
        let count = events_in_first_seconds(10.0, seconds) as f64;
        let expected = 10.0 * seconds as f64;
        assert!(
            (count - expected).abs() < expected * 0.05,
            "expected about {expected} events, got {count}"
        );
    }

    #[test]
    fn offsets_are_non_decreasing_milliseconds() {
        let clock = Clock::builder()
            .key("c".into())
            .hertz(10.0)
            .build()
            .unwrap();
        let offsets: Vec<i64> = clock.take(1000).collect();
        assert!(offsets.windows(2).all(|w| w[0] <= w[1]));
        assert!(offsets.iter().any(|o| o % 1000 != 0));
    }
}
