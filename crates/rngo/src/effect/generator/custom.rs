use super::GeneratorBuilder;
use crate::effect::generator::{
    Generator, GeneratorBuildVisitor, GeneratorParseVisitor, GeneratorParser,
};
use crate::error::{BuildErrors, ParseErrors};
use crate::spec;

pub(crate) struct CustomParser {
    name: String,
    generator_type: spec::GeneratorType,
}

impl CustomParser {
    pub(crate) fn new(name: String, generator_type: spec::GeneratorType) -> Self {
        CustomParser {
            name,
            generator_type,
        }
    }
}

impl GeneratorParser for CustomParser {
    fn key(&self) -> &str {
        &self.name
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, ParseErrors> {
        let inner = visitor
            .parse_custom_generator(self.name.clone(), self.generator_type.generator.clone())?;
        Ok(Box::new(CustomBuilder {
            name: self.name.clone(),
            inner,
        }))
    }
}

#[derive(Debug)]
struct CustomBuilder {
    name: String,
    inner: Box<dyn GeneratorBuilder>,
}

impl GeneratorBuilder for CustomBuilder {
    fn build(
        self: Box<Self>,
        visitor: GeneratorBuildVisitor,
    ) -> Result<Box<dyn Generator>, BuildErrors> {
        self.inner.build(visitor.in_custom_type(&self.name))
    }
}
