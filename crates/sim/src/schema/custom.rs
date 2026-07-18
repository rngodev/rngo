use super::SchemaBuilder;
use crate::parse::{SchemaParseVisitor, SchemaParser};
use crate::{ParseError, spec};

pub(crate) struct CustomParser {
    name: String,
    schema_type: spec::SchemaType,
}

impl CustomParser {
    pub(crate) fn new(name: String, schema_type: spec::SchemaType) -> Self {
        CustomParser { name, schema_type }
    }
}

impl SchemaParser for CustomParser {
    fn key(&self) -> &str {
        &self.name
    }

    fn parse(
        &self,
        visitor: SchemaParseVisitor,
    ) -> Result<Box<dyn SchemaBuilder>, Vec<ParseError>> {
        visitor.parse_custom_schema(self.name.clone(), self.schema_type.schema.clone())
    }
}
