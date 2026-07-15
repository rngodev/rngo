use super::SpecError;
use crate::effect::Effect;
use crate::format::{self, Format};
use crate::schema::SchemaBuilder;
use crate::simulation::{Simulation, SimulationBuilder};
use crate::util::time::Moment;
use crate::{schema, spec};
use indexmap::IndexMap;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
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

        for name in spec.schemas.keys() {
            if self.schema_parsers.iter().any(|p| p.key() == name) {
                errors.push(SpecError {
                    path: Some(vec!["schemas".into(), name.clone()]),
                    message: format!(
                        "\"{name}\" is a primitive schema type and cannot be used as a custom schema name"
                    ),
                });
            }
        }

        let custom_schema_parser = Rc::new(CustomSchemaParser::new(
            self.schema_parsers.clone(),
            spec.schemas.clone(),
        ));

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
                custom_schema_parser: Rc::clone(&custom_schema_parser),
                spec: effect.schema.clone(),
                path: vec![],
                root: vec!["effects".into(), key.clone(), "schema".into()],
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
    fn key(&self) -> &str;
    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<SpecError>>;
}

pub struct SchemaParseVisitor {
    schema_parsers: Rc<Vec<Box<dyn SchemaParser>>>,
    custom_schema_parser: Rc<CustomSchemaParser>,
    spec: super::Schema,
    path: Vec<String>,
    root: Vec<String>,
}

impl SchemaParseVisitor {
    pub fn spec(&self) -> &super::Schema {
        &self.spec
    }

    pub fn absolute_path(&self) -> Vec<String> {
        let mut path = self.root.clone();
        path.append(&mut self.path.clone());
        path
    }

    pub fn absolute_sub_path(&self, mut relative_path: Vec<String>) -> Vec<String> {
        let mut path = self.absolute_path();
        path.append(&mut relative_path);
        path
    }

    /// Shares the returned `Rc` across every reference site with the same `absolute_path()`,
    /// so a constant-only `select` embedded in a custom schema is stored once in memory no
    /// matter how many effects reference that custom schema.
    pub fn constant_select_options(&self, values: Vec<(Value, u32)>) -> ConstantSelectOptions {
        self.custom_schema_parser
            .constant_options(self.absolute_path(), values)
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
            custom_schema_parser: Rc::clone(&self.custom_schema_parser),
            spec,
            path: new_path,
            root: self.root.clone(),
        };

        child.parse()
    }

    fn parse(self) -> Result<Box<dyn SchemaBuilder>, Vec<SpecError>> {
        let parsers = Rc::clone(&self.schema_parsers);
        let matching: Vec<_> = parsers
            .iter()
            .filter(|p| self.spec.stype.as_deref() == Some(p.key()))
            .collect();

        let custom_name = self
            .spec
            .stype
            .as_deref()
            .filter(|t| self.custom_schema_parser.has(t))
            .map(str::to_string);

        let total = matching.len() + custom_name.is_some() as usize;

        if total == 0 {
            return Err(vec![SpecError {
                path: None,
                message: "no schema parser matched".into(),
            }]);
        }

        if total > 1 {
            return Err(vec![SpecError {
                path: None,
                message: format!("{total} schema parsers matched"),
            }]);
        }

        if let Some(name) = custom_name {
            let custom_schema_parser = Rc::clone(&self.custom_schema_parser);
            custom_schema_parser.resolve(&name)
        } else {
            matching.into_iter().next().unwrap().parse(self)
        }
    }
}

/// A constant select's options, shared (via `Rc`) across every reference site that parses
/// to the same absolute path.
pub type ConstantSelectOptions = Rc<Vec<(Value, u32)>>;

/// Resolves `type: <name>` references to schemas defined under `.rngo/schemas/`.
///
/// Each reference site parses its own independent `SchemaBuilder`, so behavior is
/// identical to inlining the definition literally (including independent RNG at build
/// time). This re-parses the definition per reference rather than sharing a single
/// parsed instance, which duplicates cost/memory for large schemas in general; the one
/// exception is a `select` whose options are all `constant`, which is common for large
/// enums of literal values (the original motivating case) and cheap to special-case — see
/// `constant_options` below.
struct CustomSchemaParser {
    schema_parsers: Rc<Vec<Box<dyn SchemaParser>>>,
    schema_types: IndexMap<String, super::SchemaType>,
    resolving: RefCell<Vec<String>>,
    constant_select_cache: RefCell<HashMap<Vec<String>, ConstantSelectOptions>>,
}

impl CustomSchemaParser {
    fn new(
        schema_parsers: Rc<Vec<Box<dyn SchemaParser>>>,
        schema_types: IndexMap<String, super::SchemaType>,
    ) -> Self {
        CustomSchemaParser {
            schema_parsers,
            schema_types,
            resolving: RefCell::new(Vec::new()),
            constant_select_cache: RefCell::new(HashMap::new()),
        }
    }

    fn has(&self, name: &str) -> bool {
        self.schema_types.contains_key(name)
    }

    fn constant_options(
        &self,
        path: Vec<String>,
        values: Vec<(Value, u32)>,
    ) -> ConstantSelectOptions {
        if let Some(existing) = self.constant_select_cache.borrow().get(&path) {
            return Rc::clone(existing);
        }

        let values = Rc::new(values);
        self.constant_select_cache
            .borrow_mut()
            .insert(path, Rc::clone(&values));
        values
    }

    fn resolve(self: &Rc<Self>, name: &str) -> Result<Box<dyn SchemaBuilder>, Vec<SpecError>> {
        if self.resolving.borrow().iter().any(|n| n == name) {
            let mut chain = self.resolving.borrow().clone();
            chain.push(name.to_string());
            return Err(vec![SpecError {
                path: Some(vec!["schemas".into(), name.into(), "value".into()]),
                message: format!("cyclical schema reference: {}", chain.join(" -> ")),
            }]);
        }

        let schema_type = self
            .schema_types
            .get(name)
            .unwrap_or_else(|| panic!("expected schema named {name}"))
            .clone();

        self.resolving.borrow_mut().push(name.to_string());

        let visitor = SchemaParseVisitor {
            schema_parsers: Rc::clone(&self.schema_parsers),
            custom_schema_parser: Rc::clone(self),
            spec: schema_type.schema,
            path: vec![],
            root: vec!["schemas".into(), name.into(), "value".into()],
        };

        let result = visitor.parse();
        self.resolving.borrow_mut().pop();

        result
    }
}
