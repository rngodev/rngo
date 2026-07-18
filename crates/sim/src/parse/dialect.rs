use super::format::{FormatParseContext, FormatParser};
use super::schema::{SchemaParseVisitor, SchemaParser};
use crate::effect::Effect;
use crate::schema::custom::CustomParser;
use crate::simulation::{Simulation, SimulationBuilder};
use crate::spec::{self, ParseError};
use crate::util::time::Moment;
use crate::{format, schema};
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
        let spec: spec::Simulation = spec::from_value(value)?;
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

            let visitor = SchemaParseVisitor::new(
                self.schema_parsers.clone(),
                Rc::clone(&custom_schemas),
                effect.schema.clone(),
                vec![],
                vec!["effects".into(), key.clone(), "schema".into()],
            );

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
