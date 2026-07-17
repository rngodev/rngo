use super::SchemaBuilder;
use crate::ParseError;
use crate::spec::{self, SchemaParseVisitor, SchemaParser};

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
        let type_visitor =
            visitor.enter_type(self.name.clone(), self.schema_type.schema.clone())?;
        type_visitor.parse()
    }
}
