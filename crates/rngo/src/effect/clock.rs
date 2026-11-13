use crate::cel::CelContextExt;
use crate::{BuildError, EffectKey, RunError};
use cel::{Context, Program, Value};
use rand::RngExt;
use rand_pcg::Pcg32;
use rand_seeder::Seeder;

/// Poisson event clock yielding offsets in milliseconds since simulation start; it ends when the rate drops to zero.
#[derive(Debug)]
pub(super) struct Clock {
    key: String,
    rng: Pcg32,
    rate_function: RateFunction,
    last: i64,
}

impl Clock {
    pub(super) fn builder() -> ClockBuilder {
        ClockBuilder::new()
    }

    /// The offset most recently yielded, or the start offset before the first.
    pub(super) fn last(&self) -> i64 {
        self.last
    }
}

impl Iterator for Clock {
    type Item = Result<i64, RunError>;

    fn next(&mut self) -> Option<Self::Item> {
        let u: f64 = self.rng.random();
        let rate = match self.rate_function.offset_rate(self.last as f64 / 1000.0) {
            Ok(rate) => rate,
            Err(message) => {
                return Some(Err(RunError::effect(
                    &self.key,
                    EffectKey::Trigger,
                    message,
                )));
            }
        };
        if rate.is_nan() || rate < 0.0 {
            return Some(Err(RunError::effect(
                &self.key,
                EffectKey::Trigger,
                format!("rate must be a non-negative number, got {rate}"),
            )));
        }
        let interval = (-u.ln() / rate * 1000.0).round();
        if !interval.is_finite() {
            return None;
        }
        self.last = self.last.checked_add(interval as i64)?;
        Some(Ok(self.last))
    }
}

pub(super) struct ClockBuilder {
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
    pub(super) fn new() -> Self {
        ClockBuilder {
            key: String::new(),
            seed: 1,
            rate: ClockRate::Expression("hz(1, day)".into()),
            start_offset: 0,
        }
    }

    pub(super) fn key(mut self, key: String) -> Self {
        self.key = key;
        self
    }

    pub(super) fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub(super) fn hertz(mut self, hertz: f64) -> Self {
        self.rate = ClockRate::Hertz(hertz);
        self
    }

    pub(super) fn expression(mut self, expression: String) -> Self {
        self.rate = ClockRate::Expression(expression);
        self
    }

    /// Sets the clock's starting offset in milliseconds since simulation start.
    pub(super) fn start_offset(mut self, offset: i64) -> Self {
        self.start_offset = offset;
        self
    }

    pub(super) fn build(self) -> Result<Clock, Vec<BuildError>> {
        let rng: Pcg32 = Seeder::from(&format!("{}-{}", self.seed, self.key)).into_rng();

        let rate_function = match self.rate {
            ClockRate::Hertz(hertz) => RateFunction::Fixed(hertz),
            ClockRate::Expression(expression) => {
                let program = Program::compile(&expression).map_err(|e| {
                    vec![BuildError::effect(
                        self.key.clone(),
                        EffectKey::Trigger,
                        format!(
                            "could not compile expression: {}",
                            e.errors
                                .into_iter()
                                .map(|e| e.msg)
                                .collect::<Vec<_>>()
                                .join(",")
                        ),
                    )]
                })?;

                let mut context = Context::default();
                context.with_time().with_hertz();
                let references = program.references();

                if references.variables().contains(&"offset") {
                    let _ = context.add_variable("offset", 0.0);

                    program.execute(&context).map_err(|e| {
                        vec![BuildError::effect(
                            self.key.clone(),
                            EffectKey::Trigger,
                            format!("could not execute expression: {e}"),
                        )]
                    })?;

                    RateFunction::Dynamic { program, context }
                } else {
                    let value = program.execute(&context).map_err(|e| {
                        vec![BuildError::effect(
                            self.key.clone(),
                            EffectKey::Trigger,
                            format!("could not execute expression: {e}"),
                        )]
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
                            return Err(vec![BuildError::effect(
                                self.key,
                                EffectKey::Trigger,
                                format!("rate expression non-numeric: {expression}"),
                            )]);
                        }
                    }
                }
            }
        };

        if let RateFunction::Fixed(rate) = rate_function
            && (rate.is_nan() || rate < 0.0)
        {
            return Err(vec![BuildError::effect(
                self.key,
                EffectKey::Trigger,
                format!("rate must be a non-negative number, got {rate}"),
            )]);
        }

        Ok(Clock {
            key: self.key,
            rng,
            rate_function,
            last: self.start_offset,
        })
    }
}

enum RateFunction {
    Fixed(f64),
    Dynamic {
        program: Program,
        context: Context<'static>,
    },
}

impl std::fmt::Debug for RateFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RateFunction::Fixed(rate) => f.debug_tuple("Fixed").field(rate).finish(),
            RateFunction::Dynamic { .. } => f.debug_struct("Dynamic").finish_non_exhaustive(),
        }
    }
}

impl RateFunction {
    /// The rate in hertz at `offset` seconds since simulation start, or why the expression couldn't produce one.
    fn offset_rate(&mut self, offset: f64) -> Result<f64, String> {
        match self {
            RateFunction::Fixed(rate) => Ok(*rate),
            RateFunction::Dynamic { program, context } => {
                context.with_offset(offset);
                match program.execute(context) {
                    Ok(Value::Float(num)) => Ok(num),
                    Ok(Value::Int(num)) => Ok(num as f64),
                    Ok(Value::UInt(num)) => Ok(num as f64),
                    Ok(value) => Err(format!(
                        "rate expression must return a number at offset {offset}s, got {value:?}"
                    )),
                    Err(e) => Err(format!(
                        "could not execute rate expression at offset {offset}s: {e}"
                    )),
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
        clock
            .map(Result::unwrap)
            .take_while(|&offset| offset <= seconds * 1000)
            .count()
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
        let offsets: Vec<i64> = clock.take(1000).map(Result::unwrap).collect();
        assert!(offsets.windows(2).all(|w| w[0] <= w[1]));
        assert!(offsets.iter().any(|o| o % 1000 != 0));
    }

    fn expression_clock(expression: &str) -> Clock {
        Clock::builder()
            .key("c".into())
            .expression(expression.into())
            .build()
            .unwrap()
    }

    #[test]
    fn non_numeric_rate_later_in_the_run_is_an_error() {
        let mut clock = expression_clock("offset > 0.0 ? 'oops' : 1.0");
        assert!(clock.next().unwrap().is_ok());
        let error = clock.next().unwrap().unwrap_err();
        assert!(matches!(error.key, EffectKey::Trigger));
        assert!(error.message.contains("must return a number"));
    }

    #[test]
    fn negative_rate_later_in_the_run_is_an_error() {
        let mut clock = expression_clock("offset > 0.0 ? -1.0 : 1.0");
        assert!(clock.next().unwrap().is_ok());
        assert!(clock.next().unwrap().is_err());
    }

    #[test]
    fn zero_rate_ends_the_clock() {
        let mut clock = expression_clock("offset > 0.0 ? 0.0 : 1.0");
        assert!(clock.next().unwrap().is_ok());
        assert!(clock.next().is_none());
    }

    #[test]
    fn negative_fixed_rate_is_a_build_error() {
        assert!(
            Clock::builder()
                .key("c".into())
                .hertz(-1.0)
                .build()
                .is_err()
        );
    }
}
