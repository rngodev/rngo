use super::{Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorResult};
use crate::effect::generator::GeneratorContext;
use crate::error::{BuildError, ParseError as Error};
use crate::parse::{GeneratorParseVisitor, GeneratorParser};
use rand::RngExt;
use rand_pcg::Pcg32;

#[derive(Debug)]
pub(crate) struct Number {
    rng: Pcg32,
    sampling: Sampling,
    scale: Option<u32>,
}

/// How a number generator picks each value, derived from which of `minimum`, `maximum` and `step` are set.
#[derive(Debug)]
enum Sampling {
    /// Counts from a bound by `step`; `next` is the value it yields next.
    Step { next: f64, step: f64 },
    /// Uniform over `[minimum, maximum)`.
    Range { minimum: f64, maximum: f64 },
    /// `minimum` plus the magnitude of a random finite double.
    AtLeast(f64),
    /// `maximum` minus the magnitude of a random finite double.
    AtMost(f64),
    /// A random finite double, each bit pattern equally likely.
    Any,
}

impl Number {
    pub(crate) fn builder() -> NumberBuilder {
        NumberBuilder {
            minimum: None,
            maximum: None,
            scale: None,
            step: None,
        }
    }

    pub(crate) fn parser() -> NumberParser {
        NumberParser {}
    }
}

impl Generator for Number {
    fn next(&mut self, _context: &GeneratorContext) -> GeneratorResult {
        let rng = &mut self.rng;
        let mut value = match &mut self.sampling {
            Sampling::Step { next, step } => {
                let value = *next;
                *next += *step;
                value
            }
            Sampling::Range { minimum, maximum } => rng.random_range(*minimum..*maximum),
            Sampling::AtLeast(minimum) => finite(|| *minimum + random_finite(rng).abs()),
            Sampling::AtMost(maximum) => finite(|| *maximum - random_finite(rng).abs()),
            Sampling::Any => random_finite(rng),
        };

        if let Some(scale) = self.scale {
            if scale == 0 && value.abs() < i64::MAX as f64 {
                return GeneratorResult {
                    value: Some((value as i64).into()),
                    metadata: vec![],
                };
            }
            let factor = 10f64.powi(scale as i32);
            let scaled = (value * factor).round() / factor;
            if scaled.is_finite() {
                value = scaled;
            }
        }

        GeneratorResult {
            value: Some(value.into()),
            metadata: vec![],
        }
    }
}

/// A random finite double, drawn as a random bit pattern and redrawn while it is NaN or infinite.
fn random_finite(rng: &mut Pcg32) -> f64 {
    finite(|| f64::from_bits(rng.random::<u64>()))
}

/// Calls `draw` until it returns a finite value.
fn finite(mut draw: impl FnMut() -> f64) -> f64 {
    loop {
        let value = draw();
        if value.is_finite() {
            return value;
        }
    }
}

#[derive(Debug)]
pub struct NumberBuilder {
    minimum: Option<f64>,
    maximum: Option<f64>,
    scale: Option<u32>,
    step: Option<f64>,
}

impl NumberBuilder {
    pub fn minimum(mut self, minimum: impl Into<f64>) -> Self {
        self.minimum = Some(minimum.into());
        self
    }

    pub fn maximum(mut self, maximum: impl Into<f64>) -> Self {
        self.maximum = Some(maximum.into());
        self
    }

    pub fn scale(mut self, scale: u32) -> Self {
        self.scale = Some(scale);
        self
    }

    pub fn step(mut self, step: impl Into<f64>) -> Self {
        self.step = Some(step.into());
        self
    }
}

impl GeneratorBuilder for NumberBuilder {
    fn build(&self, visitor: GeneratorBuildVisitor) -> Result<Box<dyn Generator>, Vec<BuildError>> {
        let sampling = sampling(self.minimum, self.maximum, self.step)
            .map_err(|(_, message)| vec![visitor.error(message)])?;

        Ok(Box::new(Number {
            rng: visitor.rng().0,
            sampling,
            scale: self.scale,
        }))
    }
}

/// Picks how to sample from the bounds and step, or returns the field at fault (if any) and why they don't work.
fn sampling(
    minimum: Option<f64>,
    maximum: Option<f64>,
    step: Option<f64>,
) -> Result<Sampling, (Option<&'static str>, String)> {
    if let (Some(minimum), Some(maximum)) = (minimum, maximum) {
        if minimum > maximum {
            return Err((None, "minimum is greater than maximum".into()));
        }
        if step.is_none() && minimum == maximum {
            return Err((
                None,
                "minimum must be less than maximum unless step is set".into(),
            ));
        }
        if step.is_none() && !(maximum - minimum).is_finite() {
            return Err((
                None,
                "range is too wide: maximum - minimum must be at most the largest double".into(),
            ));
        }
    }

    match (step, minimum, maximum) {
        (Some(step), Some(minimum), _) if step >= 0.0 => Ok(Sampling::Step {
            next: minimum,
            step,
        }),
        (Some(step), None, _) if step >= 0.0 => Err((
            Some("minimum"),
            "minimum must be set when step is non-negative".into(),
        )),
        (Some(step), _, Some(maximum)) => Ok(Sampling::Step {
            next: maximum,
            step,
        }),
        (Some(_), _, None) => Err((
            Some("maximum"),
            "maximum must be set when step is negative".into(),
        )),
        (None, Some(minimum), Some(maximum)) => Ok(Sampling::Range { minimum, maximum }),
        (None, Some(minimum), None) => Ok(Sampling::AtLeast(minimum)),
        (None, None, Some(maximum)) => Ok(Sampling::AtMost(maximum)),
        (None, None, None) => Ok(Sampling::Any),
    }
}

pub(crate) struct NumberParser {}

impl GeneratorParser for NumberParser {
    fn key(&self) -> &str {
        "number"
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<Error>> {
        let mut builder = Number::builder();
        let mut errors = vec![];

        if let Some(v) = visitor.spec().fields.get("minimum") {
            match v.as_f64() {
                Some(minimum) => {
                    builder = builder.minimum(minimum);
                }
                None => errors.push(visitor.input_error("minimum", "minimum must be a number")),
            }
        }

        if let Some(v) = visitor.spec().fields.get("maximum") {
            match v.as_f64() {
                Some(maximum) => {
                    builder = builder.maximum(maximum);
                }
                None => errors.push(visitor.input_error("maximum", "maximum must be a number")),
            }
        }

        if let Some(v) = visitor.spec().fields.get("scale") {
            match v.as_u64() {
                Some(scale) => {
                    builder = builder.scale(scale as u32);
                }
                None => errors
                    .push(visitor.input_error("scale", "scale must be a non-negative integer")),
            }
        }

        if let Some(v) = visitor.spec().fields.get("step") {
            match v.as_f64() {
                Some(step) => {
                    builder = builder.step(step);
                }
                None => errors.push(visitor.input_error("step", "step must be a number")),
            }
        }

        if errors.is_empty()
            && let Err((field, message)) = sampling(builder.minimum, builder.maximum, builder.step)
        {
            errors.push(match field {
                Some(field) => visitor.input_error(field, message),
                None => visitor.invalid(message),
            });
        }

        if errors.is_empty() {
            Ok(Box::new(builder))
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Sampling, finite, random_finite, sampling};
    use rand_pcg::Pcg32;

    fn sampling_of(minimum: Option<f64>, maximum: Option<f64>, step: Option<f64>) -> Sampling {
        sampling(minimum, maximum, step).unwrap()
    }

    #[test]
    fn missing_bounds_widen_to_any_finite_double() {
        assert!(matches!(sampling_of(None, None, None), Sampling::Any));
        assert!(matches!(
            sampling_of(Some(1.0), None, None),
            Sampling::AtLeast(_)
        ));
        assert!(matches!(
            sampling_of(None, Some(1.0), None),
            Sampling::AtMost(_)
        ));
        assert!(matches!(
            sampling_of(Some(0.0), Some(1.0), None),
            Sampling::Range { .. }
        ));
    }

    #[test]
    fn stepped_numbers_need_the_bound_they_count_from() {
        assert!(matches!(
            sampling_of(Some(1.0), None, Some(1.0)),
            Sampling::Step { .. }
        ));
        assert!(matches!(
            sampling_of(None, Some(1.0), Some(-1.0)),
            Sampling::Step { .. }
        ));
        assert_eq!(
            sampling(None, Some(1.0), Some(1.0)).unwrap_err().0,
            Some("minimum")
        );
        assert_eq!(
            sampling(Some(1.0), None, Some(-1.0)).unwrap_err().0,
            Some("maximum")
        );
    }

    #[test]
    fn ranges_wider_than_the_largest_double_are_rejected() {
        assert!(sampling(Some(-f64::MAX / 2.0), Some(f64::MAX / 2.0), None).is_ok());
        assert!(sampling(Some(-1e308), Some(1e308), None).is_err());
        assert!(sampling(Some(-1e308), Some(1e308), Some(1.0)).is_ok());
    }

    #[test]
    fn random_finite_doubles_include_fractions_and_stay_finite() {
        let mut rng = Pcg32::new(1, 1);
        let values: Vec<f64> = (0..10_000).map(|_| random_finite(&mut rng)).collect();
        assert!(values.iter().all(|v| v.is_finite()));
        assert!(values.iter().any(|v| v.fract() != 0.0));
        assert!(values.iter().any(|v| *v < 0.0));
    }

    #[test]
    fn one_sided_bounds_hold_near_the_largest_double() {
        let mut rng = Pcg32::new(1, 1);
        for _ in 0..10_000 {
            let above = finite(|| f64::MAX + random_finite(&mut rng).abs());
            assert!(above.is_finite() && above >= f64::MAX);
            let below = finite(|| -f64::MAX - random_finite(&mut rng).abs());
            assert!(below.is_finite() && below <= -f64::MAX);
        }
    }
}
