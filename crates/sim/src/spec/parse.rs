use crate::effect::Effect;
use crate::format::{self, Format};
use crate::schema::SchemaBuilder;
use crate::schema::custom::CustomParser;
use crate::simulation::{Simulation, SimulationBuilder};
use crate::spec::ParseError;
use crate::util::time::Moment;
use crate::{schema, spec};
use std::rc::Rc;

pub struct Dialect {
    schema_parsers: Rc<Vec<Box<dyn SchemaParser>>>,
    format_parsers: Rc<Vec<Box<dyn FormatParser>>>,
}

impl Dialect {
    pub fn new(
        schema_parsers: Vec<Box<dyn SchemaParser>>,
        format_parsers: Vec<Box<dyn FormatParser>>,
    ) -> Self {
        Dialect {
            schema_parsers: Rc::new(schema_parsers),
            format_parsers: Rc::new(format_parsers),
        }
    }

    pub fn primitive() -> Self {
        Dialect::new(
            vec![
                Box::new(schema::Array::parser()),
                Box::new(schema::Constant::parser()),
                Box::new(schema::Context::parser()),
                Box::new(schema::Function::parser()),
                Box::new(schema::Number::parser()),
                Box::new(schema::Object::parser()),
                Box::new(schema::Reference::parser()),
                Box::new(schema::Select::parser()),
                Box::new(schema::Str::parser()),
            ],
            vec![Box::new(format::SqlFormat::parser())],
        )
    }

    pub fn parse_simulation_json(
        &self,
        value: serde_json::Value,
    ) -> Result<SimulationBuilder, Vec<ParseError>> {
        let spec: spec::Simulation = super::from_value(value)?;
        self.parse_simulation(spec)
    }

    pub fn parse_simulation(
        &self,
        spec: spec::Simulation,
    ) -> Result<SimulationBuilder, Vec<ParseError>> {
        let mut errors = vec![];
        let mut simulation_builder = Simulation::builder();
        let simulation_moment_parser = Moment::parser();

        if let Some(start) = &spec.start {
            match simulation_moment_parser.parse("start", start) {
                Ok(timestamp) => {
                    simulation_builder.set_start(timestamp);
                }
                Err(mut e) => errors.append(&mut e),
            };
        };

        if let Some(end) = &spec.end {
            match simulation_moment_parser.parse("end", end) {
                Ok(timestamp) => {
                    simulation_builder.set_end(timestamp);
                }
                Err(mut e) => errors.append(&mut e),
            };
        };

        for name in spec.schemas.keys() {
            if self.schema_parsers.iter().any(|p| p.key() == name) {
                errors.push(ParseError::SchemaError {
                    path: Some(vec!["schemas".into(), name.clone()]),
                    message: format!(
                        "\"{name}\" is a primitive schema type and cannot be used as a custom schema name"
                    ),
                });
            }
        }

        let custom_schemas: Rc<Vec<CustomParser>> = Rc::new(
            spec.schemas
                .iter()
                .map(|(name, schema_type)| CustomParser::new(name.clone(), schema_type.clone()))
                .collect(),
        );

        for (key, effect) in &spec.effects {
            let mut effect_builder = Effect::builder(key.clone());
            let effect_moment_parser =
                Moment::parser().simulation(&simulation_builder.start, &simulation_builder.end);

            if let Some(start) = &effect.start {
                match effect_moment_parser.parse("start", start) {
                    Ok(timestamp) => {
                        effect_builder.set_start(timestamp);
                    }
                    Err(mut e) => errors.append(&mut e),
                };
            };

            if let Some(end) = &effect.end {
                match effect_moment_parser.parse("end", end) {
                    Ok(timestamp) => {
                        effect_builder.set_end(timestamp);
                    }
                    Err(mut e) => errors.append(&mut e),
                };
            };

            if let Some(trigger_union) = &effect.trigger {
                let trigger = match trigger_union {
                    spec::TriggerUnion::Shorthand(rate) => {
                        spec::Trigger::Clock { rate: rate.clone() }
                    }
                    spec::TriggerUnion::Full(trigger) => trigger.clone(),
                };

                match trigger {
                    spec::Trigger::Clock { rate } => effect_builder.set_trigger_expression(rate),
                    spec::Trigger::Effect { key } => effect_builder.set_trigger_effect(key),
                };
            }

            match FormatParseContext::new(spec.clone(), key.clone()) {
                Ok(ctx) => {
                    let matching: Vec<_> = self
                        .format_parsers
                        .iter()
                        .filter(|p| p.should_parse(&ctx))
                        .collect();

                    let format = match matching.as_slice() {
                        [parser] => parser.parse(ctx).map(Some),
                        [] => Ok(None),
                        _ => Err(vec![ParseError::SchemaError {
                            path: None,
                            message: format!("{} schema parsers matched", matching.len()),
                        }]),
                    }?;

                    if let Some(format) = format {
                        effect_builder.set_format(format);
                    }
                }
                Err(e) => {
                    errors.push(e);
                }
            };

            let visitor = SchemaParseVisitor {
                primitive_schema_parsers: self.schema_parsers.clone(),
                custom_schema_parsers: Rc::clone(&custom_schemas),
                spec: effect.schema.clone(),
                type_stack: vec![],
                path: vec!["effects".into(), key.clone(), "schema".into()],
            };

            match visitor.parse() {
                Ok(schema_builder) => {
                    effect_builder.set_schema(schema_builder);
                    simulation_builder.set_effect(effect_builder);
                }
                Err(mut e) => errors.append(&mut e),
            }
        }

        if !errors.is_empty() {
            Err(errors)
        } else {
            Ok(simulation_builder)
        }
    }
}

pub trait FormatParser {
    fn should_parse(&self, context: &FormatParseContext) -> bool;
    fn parse(&self, context: FormatParseContext) -> Result<Box<dyn Format>, Vec<ParseError>>;
}

pub struct FormatParseContext {
    simulation: super::Simulation,
    effect_key: String,
}

impl FormatParseContext {
    pub fn new(simulation: super::Simulation, effect_key: String) -> Result<Self, ParseError> {
        let effect = simulation
            .effects
            .get(&effect_key)
            .unwrap_or_else(|| panic!("expected effect at key {effect_key}"));

        let effect_ftype = effect.format.as_ref().and_then(|f| f.ftype.as_deref());
        let system_ftype = effect
            .system
            .as_ref()
            .and_then(|s| simulation.systems.get(s))
            .and_then(|s| s.format.as_ref())
            .and_then(|f| f.ftype.as_ref());

        if let (Some(ef), Some(sf)) = (effect_ftype, system_ftype)
            && ef != sf
        {
            return Err(ParseError::SchemaError {
                path: Some(vec![
                    "effects".into(),
                    effect_key.clone(),
                    "format".into(),
                    "type".into(),
                ]),
                message: format!(
                    "effect format type \"{ef}\" does not match system format type \"{sf}\""
                ),
            });
        }

        Ok(FormatParseContext {
            simulation,
            effect_key,
        })
    }

    pub fn effect(&self) -> &spec::Effect {
        self.simulation
            .effects
            .get(&self.effect_key)
            .unwrap_or_else(|| panic!("expected effect at key {}", self.effect_key))
    }

    pub fn effect_key(&self) -> &str {
        &self.effect_key
    }

    pub fn is_format_type(&self, ftype: &str) -> bool {
        self.format()
            .and_then(|f| f.ftype)
            .map(|ft| ft == ftype)
            .unwrap_or(false)
    }

    pub fn format(&self) -> Option<super::Format> {
        let effect = self.effect();

        let system_format = effect
            .system
            .as_ref()
            .and_then(|s| self.simulation.systems.get(s))
            .and_then(|s| s.format.as_ref());

        match (effect.format.as_ref(), system_format) {
            (Some(ef), Some(sf)) => {
                let mut merged = sf.clone();
                if ef.ftype.is_some() {
                    merged.ftype = ef.ftype.clone();
                }
                for (k, v) in &ef.fields {
                    merged.fields.insert(k.clone(), v.clone());
                }
                Some(merged)
            }
            (Some(ef), None) => Some(ef.clone()),
            (None, Some(sf)) => Some(sf.clone()),
            (None, None) => None,
        }
    }
}

pub trait SchemaParser {
    fn key(&self) -> &str;
    fn parse(&self, visitor: SchemaParseVisitor)
    -> Result<Box<dyn SchemaBuilder>, Vec<ParseError>>;
}

pub struct SchemaParseVisitor {
    primitive_schema_parsers: Rc<Vec<Box<dyn SchemaParser>>>,
    custom_schema_parsers: Rc<Vec<CustomParser>>,
    spec: super::Schema,
    type_stack: Vec<String>,
    path: Vec<String>,
}

impl SchemaParseVisitor {
    pub fn spec(&self) -> &super::Schema {
        &self.spec
    }

    pub fn path(&self) -> Vec<String> {
        self.path.clone()
    }

    pub fn enter_type(
        mut self,
        name: String,
        spec: super::Schema,
    ) -> Result<Self, Vec<ParseError>> {
        if self.type_stack.contains(&name) {
            let mut cyclic_type_stack = self.type_stack.clone();
            cyclic_type_stack.push(name.clone());
            return Err(vec![ParseError::SchemaError {
                path: Some(vec!["schemas".into(), name, "schema".into()]),
                message: format!("cyclical types: {}", cyclic_type_stack.join(" -> ")),
            }]);
        }

        let path = vec!["schemas".into(), name.clone(), "schema".into()];
        self.type_stack.push(name);
        self.path = path;
        self.spec = spec;
        Ok(self)
    }

    pub fn schema_error(&self, message: impl Into<String>) -> ParseError {
        ParseError::SchemaError {
            path: Some(self.path().clone()),
            message: message.into(),
        }
    }

    pub fn input_error(&self, input: impl Into<String>, message: impl Into<String>) -> ParseError {
        let mut path = self.path().clone();
        path.push(input.into());

        ParseError::SchemaError {
            path: Some(path),
            message: message.into(),
        }
    }

    pub fn parse_child(
        &self,
        mut path: Vec<String>,
        spec: super::Schema,
    ) -> Result<Box<dyn SchemaBuilder>, Vec<ParseError>> {
        let mut new_path = self.path.clone();
        new_path.append(&mut path);

        let child = SchemaParseVisitor {
            primitive_schema_parsers: self.primitive_schema_parsers.clone(),
            custom_schema_parsers: self.custom_schema_parsers.clone(),
            type_stack: self.type_stack.clone(),
            spec,
            path: new_path,
        };

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
