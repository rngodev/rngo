use super::GeneratorBuilder;
use crate::parse::{GeneratorParseVisitor, GeneratorParser};
use crate::{ParseError, spec};

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
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<ParseError>> {
        visitor.parse_custom_generator(self.name.clone(), self.generator_type.generator.clone())
    }
}
