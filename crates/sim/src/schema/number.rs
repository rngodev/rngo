use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaResult};
use crate::build::BuildError;
use crate::schema::SchemaContext;
use crate::spec::{SchemaParseVisitor, SchemaParser, SpecError as Error};
use rand::RngExt;
use rand_pcg::Pcg32;

#[derive(Debug)]
pub struct Number {
    rng: Pcg32,
    min: f64,
    max: f64,
    scale: Option<u32>,
    step: Option<f64>,
    current: Option<f64>,
}

impl Number {
    pub fn builder() -> NumberBuilder {
        NumberBuilder {
            min: None,
            max: None,
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
            let current = self
                .current
                .get_or_insert(if step >= 0.0 { self.min } else { self.max });
            let v = *current;
            *current += step;
            v
        } else {
            self.rng.random_range(self.min..self.max)
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
    min: Option<f64>,
    max: Option<f64>,
    scale: Option<u32>,
    step: Option<f64>,
}

impl NumberBuilder {
    pub fn min(mut self, min: impl Into<f64>) -> Self {
        self.set_min(min);
        self
    }

    pub fn set_min(&mut self, min: impl Into<f64>) -> &mut Self {
        self.min = Some(min.into());
        self
    }

    pub fn max(mut self, max: impl Into<f64>) -> Self {
        self.set_max(max);
        self
    }

    pub fn set_max(&mut self, max: impl Into<f64>) -> &mut Self {
        self.max = Some(max.into());
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

        match (self.min, self.max) {
            (Some(min), Some(max)) if min > max => {
                errors.push(visitor.error("min is greater than max"));
            }
            _ => (),
        }

        if errors.is_empty() {
            Ok(Box::new(Number {
                rng: visitor.rng(),
                min: self.min.unwrap_or(f64::MIN),
                max: self.max.unwrap_or(f64::MAX),
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

        if let Some(v) = visitor.spec().fields.get("min") {
            match v.as_f64() {
                Some(min) => {
                    builder.set_min(min);
                }
                None => errors.push(Error {
                    path: Some(visitor.absolute_sub_path(vec!["min".into()])),
                    message: "min must be a number".into(),
                }),
            }
        }

        if let Some(v) = visitor.spec().fields.get("max") {
            match v.as_f64() {
                Some(max) => {
                    builder.set_max(max);
                }
                None => errors.push(Error {
                    path: Some(visitor.absolute_sub_path(vec!["max".into()])),
                    message: "max must be a number".into(),
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
