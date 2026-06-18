use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaResult};
use crate::build::BuildError;
use crate::spec::SpecError as Error;
use crate::schema::SchemaContext;
use crate::spec::{SchemaParseVisitor, SchemaParser};
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

impl Schema for Constant {
    fn next(&mut self, _context: &SchemaContext) -> SchemaResult {
        SchemaResult::Ok {
            value: self.value.clone(),
        }
    }
}

#[derive(Debug)]
pub struct ConstantBuilder {
    value: Option<Value>,
}

impl ConstantBuilder {
    pub fn value(mut self, value: impl Into<Value>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn set_value(&mut self, value: Value) -> &mut Self {
        self.value = Some(value);
        self
    }
}

impl SchemaBuilder for ConstantBuilder {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
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

impl SchemaParser for ConstantParser {
    fn should_parse(&self, visitor: &SchemaParseVisitor) -> bool {
        visitor.spec().stype == Some("constant".into())
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
        let mut builder = Constant::builder();

        match visitor.spec().fields.get("value") {
            Some(value) => {
                builder.set_value(value.clone());
                Ok(Box::new(builder))
            }
            None => Err(vec![Error {
                path: Some(visitor.absolute_path()),
                message: "value must be specified".into(),
            }]),
        }
    }
}
