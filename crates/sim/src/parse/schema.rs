use crate::schema::SchemaBuilder;
use crate::schema::custom::CustomParser;
use crate::spec::{self, ParseError};
use std::rc::Rc;

pub trait SchemaParser {
    fn key(&self) -> &str;
    fn parse(&self, visitor: SchemaParseVisitor)
    -> Result<Box<dyn SchemaBuilder>, Vec<ParseError>>;
}

pub struct SchemaParseVisitor {
    primitive_schema_parsers: Rc<Vec<Box<dyn SchemaParser>>>,
    custom_schema_parsers: Rc<Vec<CustomParser>>,
    spec: spec::Schema,
    type_stack: Vec<String>,
    source_path: Vec<String>,
}

impl SchemaParseVisitor {
    pub(super) fn new(
        primitive_schema_parsers: Rc<Vec<Box<dyn SchemaParser>>>,
        custom_schema_parsers: Rc<Vec<CustomParser>>,
        spec: spec::Schema,
        type_stack: Vec<String>,
        source_path: Vec<String>,
    ) -> Self {
        SchemaParseVisitor {
            primitive_schema_parsers,
            custom_schema_parsers,
            spec,
            type_stack,
            source_path,
        }
    }

    pub fn spec(&self) -> &spec::Schema {
        &self.spec
    }

    pub fn source_path(&self) -> Vec<String> {
        self.source_path.clone()
    }

    pub fn schema_error(&self, message: impl Into<String>) -> ParseError {
        ParseError::SchemaError {
            path: Some(self.source_path().clone()),
            message: message.into(),
        }
    }

    pub fn input_error(&self, input: impl Into<String>, message: impl Into<String>) -> ParseError {
        let mut path = self.source_path().clone();
        path.push(input.into());

        ParseError::SchemaError {
            path: Some(path),
            message: message.into(),
        }
    }

    pub fn parse_custom_schema(
        mut self,
        name: String,
        spec: spec::Schema,
    ) -> Result<Box<dyn SchemaBuilder>, Vec<ParseError>> {
        if self.type_stack.contains(&name) {
            let mut cyclic_type_stack = self.type_stack.clone();
            cyclic_type_stack.push(name.clone());
            return Err(vec![ParseError::SchemaError {
                path: Some(vec!["schemas".into(), name, "schema".into()]),
                message: format!("cyclical types: {}", cyclic_type_stack.join(" -> ")),
            }]);
        }

        self.type_stack.push(name.clone());
        self.source_path = vec!["schemas".into(), name, "schema".into()];
        self.spec = spec;

        self.parse()
    }

    pub fn parse_input_schema(
        &self,
        mut path: Vec<String>,
        spec: spec::Schema,
    ) -> Result<Box<dyn SchemaBuilder>, Vec<ParseError>> {
        let mut new_path = self.source_path.clone();
        new_path.append(&mut path);

        let child = SchemaParseVisitor::new(
            self.primitive_schema_parsers.clone(),
            self.custom_schema_parsers.clone(),
            spec,
            self.type_stack.clone(),
            new_path,
        );

        child.parse()
    }

    pub fn parse(self) -> Result<Box<dyn SchemaBuilder>, Vec<ParseError>> {
        let schema_parsers = Rc::clone(&self.primitive_schema_parsers);
        let custom_schemas = Rc::clone(&self.custom_schema_parsers);

        let matching: Vec<&dyn SchemaParser> = schema_parsers
            .iter()
            .map(|p| p.as_ref())
            .chain(custom_schemas.iter().map(|s| s as &dyn SchemaParser))
            .filter(|p| self.spec.stype.as_deref() == Some(p.key()))
            .collect();

        match matching.len() {
            0 => Err(vec![ParseError::SchemaError {
                path: None,
                message: "no schema parser matched".into(),
            }]),
            1 => matching[0].parse(self),
            total => Err(vec![ParseError::SchemaError {
                path: None,
                message: format!("{total} schema parsers matched"),
            }]),
        }
    }
}
