use super::{Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorResult};
use crate::build::BuildError;
use crate::effect::generator::GeneratorContext;
use crate::parse::{GeneratorParseVisitor, GeneratorParser};
use crate::spec::ParseError as Error;
use rand::RngExt;
use rand_pcg::Pcg32;

#[derive(Debug)]
pub struct Number {
    rng: Pcg32,
    minimum: f64,
    maximum: f64,
    scale: Option<u32>,
    step: Option<f64>,
    current: Option<f64>,
}

impl Number {
    pub fn builder() -> NumberBuilder {
        NumberBuilder {
            minimum: None,
            maximum: None,
            scale: None,
            step: None,
        }
    }

    pub fn parser() -> NumberParser {
        NumberParser {}
    }
}

impl Generator for Number {
    fn next(&mut self, _context: &GeneratorContext) -> GeneratorResult {
        let mut value = if let Some(step) = self.step {
            let current = self.current.get_or_insert(if step >= 0.0 {
                self.minimum
            } else {
                self.maximum
            });
            let v = *current;
            *current += step;
            v
        } else {
            self.rng.random_range(self.minimum..self.maximum)
        };

        if let Some(scale) = self.scale {
            if scale == 0 {
                return GeneratorResult {
                    value: Some((value as i64).into()),
                    metadata: vec![],
                };
            } else {
                let factor = 10f64.powi(scale as i32);
                value = (value * factor).round() / factor;
            }
        }

        GeneratorResult {
            value: Some(value.into()),
            metadata: vec![],
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
        self.set_minimum(minimum);
        self
    }

    pub fn set_minimum(&mut self, minimum: impl Into<f64>) -> &mut Self {
        self.minimum = Some(minimum.into());
        self
    }

    pub fn maximum(mut self, maximum: impl Into<f64>) -> Self {
        self.set_maximum(maximum);
        self
    }

    pub fn set_maximum(&mut self, maximum: impl Into<f64>) -> &mut Self {
        self.maximum = Some(maximum.into());
        self
    }

    pub fn scale(mut self, scale: u32) -> Self {
        self.set_scale(scale);
        self
    }

    pub fn set_scale(&mut self, scale: u32) -> &mut Self {
        self.scale = Some(scale);
        self
    }

    pub fn step(mut self, step: impl Into<f64>) -> Self {
        self.set_step(step);
        self
    }

    pub fn set_step(&mut self, step: impl Into<f64>) -> &mut Self {
        self.step = Some(step.into());
        self
    }
}

impl GeneratorBuilder for NumberBuilder {
    fn build(&self, visitor: GeneratorBuildVisitor) -> Result<Box<dyn Generator>, Vec<BuildError>> {
        if let (Some(minimum), Some(maximum)) = (self.minimum, self.maximum) {
            if minimum > maximum {
                return Err(vec![visitor.error("minimum is greater than maximum")]);
            }
            if minimum == maximum && self.step.is_none() {
                return Err(vec![
                    visitor.error("minimum must be less than maximum unless step is set"),
                ]);
            }
        }

        if let Some(bound) = missing_bound(self.minimum, self.maximum, self.step) {
            return Err(vec![visitor.error(format!("{bound} must be set"))]);
        }

        Ok(Box::new(Number {
            rng: visitor.rng(),
            minimum: self.minimum.unwrap_or(f64::NAN),
            maximum: self.maximum.unwrap_or(f64::NAN),
            scale: self.scale,
            step: self.step,
            current: None,
        }))
    }
}

/// The first bound a number generator needs but lacks: both without `step`, else the one `step` counts from.
fn missing_bound(
    minimum: Option<f64>,
    maximum: Option<f64>,
    step: Option<f64>,
) -> Option<&'static str> {
    match step {
        Some(step) if step >= 0.0 => minimum.is_none().then_some("minimum"),
        Some(_) => maximum.is_none().then_some("maximum"),
        None if minimum.is_none() => Some("minimum"),
        None => maximum.is_none().then_some("maximum"),
    }
}

pub struct NumberParser {}

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
                    builder.set_minimum(minimum);
                }
                None => errors.push(visitor.input_error("minimum", "minimum must be a number")),
            }
        }

        if let Some(v) = visitor.spec().fields.get("maximum") {
            match v.as_f64() {
                Some(maximum) => {
                    builder.set_maximum(maximum);
                }
                None => errors.push(visitor.input_error("maximum", "maximum must be a number")),
            }
        }

        if let Some(v) = visitor.spec().fields.get("scale") {
            match v.as_u64() {
                Some(scale) => {
                    builder.set_scale(scale as u32);
                }
                None => errors
                    .push(visitor.input_error("scale", "scale must be a non-negative integer")),
            }
        }

        if let Some(v) = visitor.spec().fields.get("step") {
            match v.as_f64() {
                Some(step) => {
                    builder.set_step(step);
                }
                None => errors.push(visitor.input_error("step", "step must be a number")),
            }
        }

        if errors.is_empty()
            && let Some(bound) = missing_bound(builder.minimum, builder.maximum, builder.step)
        {
            errors.push(visitor.input_error(bound, format!("{bound} must be set")));
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
    use super::missing_bound;

    #[test]
    fn random_numbers_need_both_bounds() {
        assert_eq!(missing_bound(None, None, None), Some("minimum"));
        assert_eq!(missing_bound(Some(0.0), None, None), Some("maximum"));
        assert_eq!(missing_bound(None, Some(1.0), None), Some("minimum"));
        assert_eq!(missing_bound(Some(0.0), Some(1.0), None), None);
    }

    #[test]
    fn stepped_numbers_need_only_the_bound_they_count_from() {
        assert_eq!(missing_bound(Some(1.0), None, Some(1.0)), None);
        assert_eq!(missing_bound(None, Some(1.0), Some(1.0)), Some("minimum"));
        assert_eq!(missing_bound(None, Some(1.0), Some(-1.0)), None);
        assert_eq!(missing_bound(Some(1.0), None, Some(-1.0)), Some("maximum"));
    }
}
