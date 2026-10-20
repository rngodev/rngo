use crate::effect::generator::GeneratorBuilder;
use crate::effect::generator::custom::CustomParser;
use crate::spec::{self, ParseError};
use std::rc::Rc;

pub trait GeneratorParser {
    fn key(&self) -> &str;
    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<ParseError>>;
}

pub struct GeneratorParseVisitor {
    primitive_generator_parsers: Rc<Vec<Box<dyn GeneratorParser>>>,
    custom_generator_parsers: Rc<Vec<CustomParser>>,
    spec: spec::Generator,
    type_stack: Vec<String>,
    source_path: Vec<String>,
}

impl GeneratorParseVisitor {
    pub(super) fn new(
        primitive_generator_parsers: Rc<Vec<Box<dyn GeneratorParser>>>,
        custom_generator_parsers: Rc<Vec<CustomParser>>,
        spec: spec::Generator,
        type_stack: Vec<String>,
        source_path: Vec<String>,
    ) -> Self {
        GeneratorParseVisitor {
            primitive_generator_parsers,
            custom_generator_parsers,
            spec,
            type_stack,
            source_path,
        }
    }

    pub fn spec(&self) -> &spec::Generator {
        &self.spec
    }

    pub fn source_path(&self) -> Vec<String> {
        self.source_path.clone()
    }

    pub fn invalid(&self, message: impl Into<String>) -> ParseError {
        ParseError::Invalid {
            path: Some(self.source_path().clone()),
            message: message.into(),
        }
    }

    pub fn input_error(&self, input: impl Into<String>, message: impl Into<String>) -> ParseError {
        let mut path = self.source_path().clone();
        path.push(input.into());

        ParseError::Invalid {
            path: Some(path),
            message: message.into(),
        }
    }

    pub fn parse_custom_generator(
        mut self,
        name: String,
        spec: spec::Generator,
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<ParseError>> {
        if self.type_stack.contains(&name) {
            let mut cyclic_type_stack = self.type_stack.clone();
            cyclic_type_stack.push(name.clone());
            return Err(vec![ParseError::Invalid {
                path: Some(vec!["generators".into(), name, "generator".into()]),
                message: format!("cyclical types: {}", cyclic_type_stack.join(" -> ")),
            }]);
        }

        self.type_stack.push(name.clone());
        self.source_path = vec!["generators".into(), name, "generator".into()];
        self.spec = spec;

        self.parse()
    }

    pub fn parse_input_generator(
        &self,
        mut path: Vec<String>,
        spec: spec::Generator,
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<ParseError>> {
        let mut new_path = self.source_path.clone();
        new_path.append(&mut path);

        let child = GeneratorParseVisitor::new(
            self.primitive_generator_parsers.clone(),
            self.custom_generator_parsers.clone(),
            spec,
            self.type_stack.clone(),
            new_path,
        );

        child.parse()
    }

    pub fn parse(self) -> Result<Box<dyn GeneratorBuilder>, Vec<ParseError>> {
        let generator_parsers = Rc::clone(&self.primitive_generator_parsers);
        let custom_generators = Rc::clone(&self.custom_generator_parsers);

        let matching: Vec<&dyn GeneratorParser> = generator_parsers
            .iter()
            .map(|p| p.as_ref())
            .chain(custom_generators.iter().map(|s| s as &dyn GeneratorParser))
            .filter(|p| self.spec.stype.as_deref() == Some(p.key()))
            .collect();

        match matching.len() {
            0 => Err(vec![ParseError::Invalid {
                path: None,
                message: "no generator parser matched".into(),
            }]),
            1 => matching[0].parse(self),
            total => Err(vec![ParseError::Invalid {
                path: None,
                message: format!("{total} generator parsers matched"),
            }]),
        }
    }
}
