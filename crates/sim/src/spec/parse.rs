use super::SpecError;
use crate::effect::Effect;
use crate::format::{self, Format};
use crate::schema::SchemaBuilder;
use crate::simulation::{Simulation, SimulationBuilder};
use crate::util::time::Moment;
use crate::{schema, spec};
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

        let custom_schema_state = Rc::new(CustomSchemaState {
            call_stack: RefCell::new(Vec::new()),
            constant_select_cache: RefCell::new(HashMap::new()),
        });

        let custom_schemas: Rc<Vec<CustomSchema>> = Rc::new(
            spec.schemas
                .iter()
                .map(|(name, schema_type)| CustomSchema {
                    name: name.clone(),
                    schema_type: schema_type.clone(),
                    state: Rc::clone(&custom_schema_state),
                })
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
                primitive_schema_parsers: self.schema_parsers.clone(),
                custom_schema_parsers: Rc::clone(&custom_schemas),
                custom_schema_state: None,
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
    primitive_schema_parsers: Rc<Vec<Box<dyn SchemaParser>>>,
    custom_schema_parsers: Rc<Vec<CustomSchema>>,
    /// Only present while parsing within a custom schema's own body (i.e. once dispatch has
    /// entered `CustomSchema::parse`); `None` for an effect's own top-level schema.
    custom_schema_state: Option<Rc<CustomSchemaState>>,
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

    /// `Some` only while parsing within a custom schema's own body (see
    /// `CustomSchemaState::constant_select_options`); `None` for an effect's own top-level
    /// schema, which is only ever parsed once and so has nothing to share/cache.
    pub(crate) fn custom_schema_state(&self) -> Option<&CustomSchemaState> {
        self.custom_schema_state.as_deref()
    }

    pub fn parse_child(
        &self,
        mut path: Vec<String>,
        spec: super::Schema,
    ) -> Result<Box<dyn SchemaBuilder>, Vec<SpecError>> {
        let mut new_path = self.path.clone();
        new_path.append(&mut path);

        let child = SchemaParseVisitor {
            primitive_schema_parsers: self.primitive_schema_parsers.clone(),
            custom_schema_parsers: self.custom_schema_parsers.clone(),
            custom_schema_state: self.custom_schema_state.clone(),
            spec,
            path: new_path,
            root: self.root.clone(),
        };

        child.parse()
    }

    fn parse(self) -> Result<Box<dyn SchemaBuilder>, Vec<SpecError>> {
        let schema_parsers = Rc::clone(&self.primitive_schema_parsers);
        let custom_schemas = Rc::clone(&self.custom_schema_parsers);

        let matching: Vec<&dyn SchemaParser> = schema_parsers
            .iter()
            .map(|p| p.as_ref())
            .chain(custom_schemas.iter().map(|s| s as &dyn SchemaParser))
            .filter(|p| self.spec.stype.as_deref() == Some(p.key()))
            .collect();

        match matching.len() {
            0 => Err(vec![SpecError {
                path: None,
                message: "no schema parser matched".into(),
            }]),
            1 => matching[0].parse(self),
            total => Err(vec![SpecError {
                path: None,
                message: format!("{total} schema parsers matched"),
            }]),
        }
    }
}

/// A constant select's options, shared (via `Rc`) across every reference site that parses
/// to the same absolute path.
pub type ConstantSelectOptions = Rc<Vec<(Value, u32)>>;

/// Resolves `type: <name>` references to schemas defined under `.rngo/schemas/`: wraps a
/// custom schema's definition together with its name so it can act as a `SchemaParser`,
/// matched and dispatched the same way as primitive ones.
///
/// Each reference site parses its own independent `SchemaBuilder`, so behavior is
/// identical to inlining the definition literally (including independent RNG at build
/// time). This re-parses the definition per reference rather than sharing a single
/// parsed instance, which duplicates cost/memory for large schemas in general; the one
/// exception is a `select` whose options are all `constant`, which is common for large
/// enums of literal values (the original motivating case) and cheap to special-case — see
/// `constant_select_options` above.
struct CustomSchema {
    name: String,
    schema_type: super::SchemaType,
    state: Rc<CustomSchemaState>,
}

/// Cycle-detection stack and constant-`select` cache shared by every `CustomSchema`, so both
/// stay correct/shared regardless of which effect or custom schema first references a name.
pub(crate) struct CustomSchemaState {
    call_stack: RefCell<Vec<String>>,
    constant_select_cache: RefCell<HashMap<Vec<String>, ConstantSelectOptions>>,
}

impl CustomSchemaState {
    /// Shares the returned `Rc` across every reference site with the same `path`, so a
    /// constant-only `select` embedded in a custom schema is stored once in memory no matter
    /// how many effects reference that custom schema.
    pub(crate) fn constant_select_options(
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
}

impl SchemaParser for CustomSchema {
    fn key(&self) -> &str {
        &self.name
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<SpecError>> {
        if self
            .state
            .call_stack
            .borrow()
            .iter()
            .any(|n| n == &self.name)
        {
            let mut chain = self.state.call_stack.borrow().clone();
            chain.push(self.name.clone());
            return Err(vec![SpecError {
                path: Some(vec!["schemas".into(), self.name.clone(), "value".into()]),
                message: format!("cyclical schema reference: {}", chain.join(" -> ")),
            }]);
        }

        self.state.call_stack.borrow_mut().push(self.name.clone());

        let child = SchemaParseVisitor {
            primitive_schema_parsers: visitor.primitive_schema_parsers.clone(),
            custom_schema_parsers: visitor.custom_schema_parsers.clone(),
            custom_schema_state: Some(Rc::clone(&self.state)),
            spec: self.schema_type.schema.clone(),
            path: vec![],
            root: vec!["schemas".into(), self.name.clone(), "value".into()],
        };

        let result = child.parse();
        self.state.call_stack.borrow_mut().pop();

        result
    }
}
