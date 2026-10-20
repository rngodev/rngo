use super::{Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorResult};
use crate::build::BuildError;
use crate::effect::generator::GeneratorContext;
use crate::parse::{GeneratorParseVisitor, GeneratorParser};
use crate::spec::ParseError;
use serde_json::Value;

#[derive(Debug)]
pub struct Constant {
    value: Value,
}

impl Constant {
    pub fn builder() -> ConstantBuilder {
        ConstantBuilder { value: None }
    }

    pub fn parser() -> ConstantParser {
        ConstantParser {}
    }
}

impl Generator for Constant {
    fn next(&mut self, _context: &GeneratorContext) -> GeneratorResult {
        self.value.clone().into()
    }
}

#[derive(Debug)]
pub struct ConstantBuilder {
    value: Option<Value>,
}

impl ConstantBuilder {
    pub fn value(mut self, value: impl Into<Value>) -> Self {
        self.set_value(value);
        self
    }

    pub fn set_value(&mut self, value: impl Into<Value>) -> &mut Self {
        self.value = Some(value.into());
        self
    }
}

impl GeneratorBuilder for ConstantBuilder {
    fn build(&self, visitor: GeneratorBuildVisitor) -> Result<Box<dyn Generator>, Vec<BuildError>> {
        if let Some(value) = &self.value {
            Ok(Box::new(Constant {
                value: value.clone(),
            }))
        } else {
            Err(vec![visitor.error("value was not set")])
        }
    }
}

pub struct ConstantParser {}

impl GeneratorParser for ConstantParser {
    fn key(&self) -> &str {
        "constant"
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<ParseError>> {
        let mut builder = Constant::builder();

        match visitor.spec().fields.get("value") {
            Some(value) => {
                builder.set_value(value.clone());
                Ok(Box::new(builder))
            }
            None => Err(vec![visitor.invalid("value must be specified")]),
        }
    }
}
