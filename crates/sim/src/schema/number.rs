use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaResult};
use crate::build::BuildError;
use crate::schema::SchemaContext;
use crate::spec::{SchemaParseVisitor, SchemaParser, SpecError as Error};
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

impl Schema for Number {
    fn next(&mut self, _context: &SchemaContext) -> SchemaResult {
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
                return SchemaResult::Ok {
                    value: (value as i64).into(),
                };
            } else {
                let factor = 10f64.powi(scale as i32);
                value = (value * factor).round() / factor;
            }
        }

        SchemaResult::Ok {
            value: value.into(),
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

impl SchemaBuilder for NumberBuilder {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
        let mut errors = vec![];

        match (self.minimum, self.maximum) {
            (Some(minimum), Some(maximum)) if minimum > maximum => {
                errors.push(visitor.error("minimum is greater than maximum"));
            }
            _ => (),
        }

        if errors.is_empty() {
            Ok(Box::new(Number {
                rng: visitor.rng(),
                minimum: self.minimum.unwrap_or(f64::MIN),
                maximum: self.maximum.unwrap_or(f64::MAX),
                scale: self.scale,
                step: self.step,
                current: None,
            }))
        } else {
            Err(errors)
        }
    }
}

pub struct NumberParser {}

impl SchemaParser for NumberParser {
    fn should_parse(&self, visitor: &SchemaParseVisitor) -> bool {
        visitor.spec().stype == Some("number".into())
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
        let mut builder = Number::builder();
        let mut errors = vec![];

        if let Some(v) = visitor.spec().fields.get("minimum") {
            match v.as_f64() {
                Some(minimum) => {
                    builder.set_minimum(minimum);
                }
                None => errors.push(Error {
                    path: Some(visitor.absolute_sub_path(vec!["minimum".into()])),
                    message: "minimum must be a number".into(),
                }),
            }
        }

        if let Some(v) = visitor.spec().fields.get("maximum") {
            match v.as_f64() {
                Some(maximum) => {
                    builder.set_maximum(maximum);
                }
                None => errors.push(Error {
                    path: Some(visitor.absolute_sub_path(vec!["maximum".into()])),
                    message: "maximum must be a number".into(),
                }),
            }
        }

        if let Some(v) = visitor.spec().fields.get("scale") {
            match v.as_u64() {
                Some(scale) => {
                    builder.set_scale(scale as u32);
                }
                None => errors.push(Error {
                    path: Some(visitor.absolute_sub_path(vec!["scale".into()])),
                    message: "scale must be a non-negative integer".into(),
                }),
            }
        }

        if let Some(v) = visitor.spec().fields.get("step") {
            match v.as_f64() {
                Some(step) => {
                    builder.set_step(step);
                }
                None => errors.push(Error {
                    path: Some(visitor.absolute_sub_path(vec!["step".into()])),
                    message: "step must be a number".into(),
                }),
            }
        }

        if errors.is_empty() {
            Ok(Box::new(builder))
        } else {
            Err(errors)
        }
    }
}
