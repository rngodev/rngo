use crate::effect::generator::GeneratorBuilder;
use crate::effect::generator::custom::CustomParser;
use crate::error::{ParseError, ParseErrors};
use crate::spec;
use std::rc::Rc;

pub trait GeneratorParser {
    fn key(&self) -> &str;
    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, ParseErrors>;
}

pub struct GeneratorParseVisitor {
    primitive_generator_parsers: Rc<Vec<Box<dyn GeneratorParser>>>,
    custom_generator_parsers: Rc<Vec<CustomParser>>,
    spec: spec::Generator,
    type_stack: Vec<String>,
    source_path: Vec<String>,
}

impl GeneratorParseVisitor {
    pub(crate) fn new(
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

    /// An error located at this generator's spec.
    pub fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError::invalid(message).at(self.source_path())
    }

    /// An error located at `field` of this generator's spec.
    pub fn field_error(&self, field: impl Into<String>, message: impl Into<String>) -> ParseError {
        let mut path = self.source_path();
        path.push(field.into());

        ParseError::invalid(message).at(path)
    }

    pub(crate) fn parse_custom_generator(
        mut self,
        name: String,
        spec: spec::Generator,
    ) -> Result<Box<dyn GeneratorBuilder>, ParseErrors> {
        if self.type_stack.contains(&name) {
            let mut cyclic_type_stack = self.type_stack.clone();
            cyclic_type_stack.push(name.clone());
            return Err(ParseError::invalid(format!(
                "cyclical types: {}",
                cyclic_type_stack.join(" -> ")
            ))
            .at(["generators", name.as_str(), "generator"])
            .into());
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
    ) -> Result<Box<dyn GeneratorBuilder>, ParseErrors> {
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

    pub(crate) fn parse(self) -> Result<Box<dyn GeneratorBuilder>, ParseErrors> {
        let generator_parsers = Rc::clone(&self.primitive_generator_parsers);
        let custom_generators = Rc::clone(&self.custom_generator_parsers);

        let matching: Vec<&dyn GeneratorParser> = generator_parsers
            .iter()
            .map(|p| p.as_ref())
            .chain(custom_generators.iter().map(|s| s as &dyn GeneratorParser))
            .filter(|p| self.spec.stype.as_deref() == Some(p.key()))
            .collect();

        match matching.len() {
            0 => Err(ParseError::invalid("no generator parser matched").into()),
            1 => matching[0].parse(self),
            total => Err(ParseError::invalid(format!("{total} generator parsers matched")).into()),
        }
    }
}
