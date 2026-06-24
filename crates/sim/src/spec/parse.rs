use super::SpecError;
use crate::effect::Effect;
use crate::format::{self, Format};
use crate::schema;
use crate::schema::SchemaBuilder;
use crate::simulation::{Simulation, SimulationBuilder};
use crate::spec;
use crate::util::time::Moment;
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

    pub fn core() -> Self {
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
    ) -> Result<SimulationBuilder, Vec<SpecError>> {
        let spec: spec::Simulation = super::from_value(value)?;
        self.parse_simulation(spec)
    }

    pub fn parse_simulation(
        &self,
        spec: spec::Simulation,
    ) -> Result<SimulationBuilder, Vec<SpecError>> {
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
                        effect_builder.set_start(timestamp);
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
                        _ => Err(vec![SpecError {
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
                schema_parsers: self.schema_parsers.clone(),
                simulation_seed: simulation_builder.seed,
                effect_key: effect_builder.key.clone(),
                spec: effect.schema.clone(),
                path: vec![],
            };

            match visitor.parse() {
                Ok(schema_builder) => {
                    effect_builder.set_schema(schema_builder);
                    simulation_builder.add_effect(effect_builder);
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
    fn parse(&self, context: FormatParseContext) -> Result<Box<dyn Format>, Vec<SpecError>>;
}

pub struct FormatParseContext {
    simulation: super::Simulation,
    effect_key: String,
}

impl FormatParseContext {
    pub fn new(simulation: super::Simulation, effect_key: String) -> Result<Self, SpecError> {
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
            return Err(SpecError {
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
    fn should_parse(&self, visitor: &SchemaParseVisitor) -> bool;
    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<SpecError>>;
}

pub struct SchemaParseVisitor {
    schema_parsers: Rc<Vec<Box<dyn SchemaParser>>>,
    simulation_seed: u64,
    effect_key: String,
    spec: super::Schema,
    path: Vec<String>,
}

impl SchemaParseVisitor {
    pub fn spec(&self) -> &super::Schema {
        &self.spec
    }

    pub fn absolute_path(&self) -> Vec<String> {
        let mut path = vec![
            "effects".to_string(),
            self.effect_key.to_string(),
            "schema".to_string(),
        ];
        path.append(&mut self.path.clone());
        path
    }

    pub fn absolute_sub_path(&self, mut relative_path: Vec<String>) -> Vec<String> {
        let mut path = self.absolute_path();
        path.append(&mut relative_path);
        path
    }

    pub fn parse_child(
        &self,
        mut path: Vec<String>,
        spec: super::Schema,
    ) -> Result<Box<dyn SchemaBuilder>, Vec<SpecError>> {
        let mut new_path = self.path.clone();
        new_path.append(&mut path);

        let child = SchemaParseVisitor {
            schema_parsers: self.schema_parsers.clone(),
            simulation_seed: self.simulation_seed,
            effect_key: self.effect_key.clone(),
            spec,
            path: new_path,
        };

        child.parse()
    }

    fn parse(self) -> Result<Box<dyn SchemaBuilder>, Vec<SpecError>> {
        let parsers = Rc::clone(&self.schema_parsers);
        let matching: Vec<_> = parsers.iter().filter(|p| p.should_parse(&self)).collect();

        match matching.as_slice() {
            [parser] => parser.parse(self),
            [] => Err(vec![SpecError {
                path: None,
                message: "no schema parser matched".into(),
            }]),
            _ => Err(vec![SpecError {
                path: None,
                message: format!("{} schema parsers matched", matching.len()),
            }]),
        }
    }
}
