use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult};
use crate::build::{BuildError, SchemaEdge};
use crate::spec::{self, SchemaParseVisitor, SchemaParser, SpecError as Error};
use rand::RngExt;
use rand_pcg::Pcg32;
use serde::Deserialize;
use std::rc::Rc;

#[derive(Debug)]
pub struct Select {
    rng: Pcg32,
    options: SelectOptions,
}

impl Select {
    pub fn builder() -> SelectBuilder {
        SelectBuilder {
            option_builders: vec![],
        }
    }

    pub fn parser() -> SelectParser {
        SelectParser {}
    }
}

#[derive(Debug)]
enum SelectOptions {
    /// Shared across every reference site to the custom schema this select was parsed from
    /// (see `SchemaParseVisitor::constant_select_options`), so a large enum of literal values
    /// is stored once in memory rather than once per reference site.
    Constants(spec::ConstantSelectOptions),
    Schemas(Vec<SelectProperty>),
}

#[derive(Debug)]
pub struct SelectProperty {
    pub schema: Box<dyn Schema>,
    pub weight: u32,
}

impl Schema for Select {
    fn next(&mut self, context: &SchemaContext) -> SchemaResult {
        match &mut self.options {
            SelectOptions::Constants(options) => {
                let total_weight: u32 = options.iter().map(|(_, weight)| weight).sum();
                let mut chosen = self.rng.random_range(0..total_weight);

                for (value, weight) in options.iter() {
                    if chosen < *weight {
                        return SchemaResult::Ok {
                            value: value.clone(),
                        };
                    }
                    chosen -= weight;
                }

                SchemaResult::Err("no streams available".into())
            }
            SelectOptions::Schemas(properties) => {
                let total_weight: u32 = properties.iter().map(|p| p.weight).sum();
                let mut chosen = self.rng.random_range(0..total_weight);

                for property in properties {
                    if chosen < property.weight {
                        return property.schema.next(context);
                    }
                    chosen -= property.weight;
                }

                SchemaResult::Err("no streams available".into())
            }
        }
    }
}

#[derive(Debug)]
pub struct SelectBuilder {
    option_builders: Vec<(u32, Box<dyn SchemaBuilder>)>,
}

impl SelectBuilder {
    pub fn option(mut self, weight: u32, builder: impl SchemaBuilder + 'static) -> Self {
        self.set_option(weight, builder);
        self
    }

    pub fn set_option(&mut self, weight: u32, builder: impl SchemaBuilder + 'static) -> &mut Self {
        self.option_builders.push((weight, Box::new(builder)));
        self
    }
}

impl SchemaBuilder for SelectBuilder {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
        if self.option_builders.is_empty() {
            return Err(vec![visitor.error("options must not be empty")]);
        }

        let mut errors = vec![];
        let mut properties = vec![];

        for (i, (weight, builder)) in self.option_builders.iter().enumerate() {
            let option_visitor = visitor.follow_edge(SchemaEdge {
                kind: "option",
                key: i.to_string(),
            });
            match builder.build(option_visitor) {
                Ok(schema) => properties.push(SelectProperty {
                    schema,
                    weight: *weight,
                }),
                Err(mut e) => errors.append(&mut e),
            }
        }

        if errors.is_empty() {
            Ok(Box::new(Select {
                rng: visitor.rng(),
                options: SelectOptions::Schemas(properties),
            }))
        } else {
            Err(errors)
        }
    }
}

#[derive(Debug)]
struct ConstantSelectBuilder {
    options: spec::ConstantSelectOptions,
}

impl SchemaBuilder for ConstantSelectBuilder {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
        Ok(Box::new(Select {
            rng: visitor.rng(),
            options: SelectOptions::Constants(Rc::clone(&self.options)),
        }))
    }
}

#[derive(Deserialize)]
struct OptionSpec {
    weight: Option<u32>,
    schema: spec::Schema,
}

pub struct SelectParser {}

impl SchemaParser for SelectParser {
    fn key(&self) -> &str {
        "select"
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
        let options_value = match visitor.spec().fields.get("options") {
            Some(v) => v.clone(),
            None => {
                return Err(vec![Error {
                    path: Some(visitor.absolute_path()),
                    message: "options must be specified".into(),
                }]);
            }
        };

        let option_specs: Vec<OptionSpec> = serde_json::from_value(options_value).map_err(|e| {
            vec![Error {
                path: Some(visitor.absolute_sub_path(vec!["options".into()])),
                message: format!("options parsing failed: {e}"),
            }]
        })?;

        if option_specs.is_empty() {
            return Err(vec![Error {
                path: Some(visitor.absolute_path()),
                message: "options must not be empty".into(),
            }]);
        }

        if option_specs
            .iter()
            .all(|option| option.schema.stype.as_deref() == Some("constant"))
        {
            return parse_constants(&visitor, option_specs);
        }

        let mut errors = vec![];
        let mut builder = Select::builder();

        for (i, option) in option_specs.into_iter().enumerate() {
            let path = vec!["options".into(), i.to_string(), "schema".into()];
            match visitor.parse_child(path, option.schema) {
                Ok(schema_builder) => {
                    builder.set_option(option.weight.unwrap_or(1), schema_builder);
                }
                Err(mut e) => errors.append(&mut e),
            }
        }

        if errors.is_empty() {
            Ok(Box::new(builder))
        } else {
            Err(errors)
        }
    }
}

fn parse_constants(
    visitor: &SchemaParseVisitor,
    option_specs: Vec<OptionSpec>,
) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
    let mut errors = vec![];
    let mut values = vec![];

    for (i, option) in option_specs.iter().enumerate() {
        match option.schema.fields.get("value") {
            Some(value) => values.push((value.clone(), option.weight.unwrap_or(1))),
            None => errors.push(Error {
                path: Some(visitor.absolute_sub_path(vec![
                    "options".into(),
                    i.to_string(),
                    "schema".into(),
                    "value".into(),
                ])),
                message: "value must be specified".into(),
            }),
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let options = visitor.constant_select_options(values);

    Ok(Box::new(ConstantSelectBuilder { options }))
}
