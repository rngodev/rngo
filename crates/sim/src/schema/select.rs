use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult};
use crate::build::{BuildError, SchemaEdge};
use crate::spec::{self, ParseError as Error, SchemaParseVisitor, SchemaParser};
use rand::RngExt;
use rand_pcg::Pcg32;
use serde::Deserialize;

#[derive(Debug)]
pub struct Select {
    rng: Pcg32,
    properties: Vec<SelectProperty>,
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
pub struct SelectProperty {
    pub schema: Box<dyn Schema>,
    pub weight: u32,
}

impl Schema for Select {
    fn next(&mut self, context: &SchemaContext) -> SchemaResult {
        let total_weight: u32 = self.properties.iter().map(|p| p.weight).sum();
        let mut chosen = self.rng.random_range(0..total_weight);

        for property in &mut self.properties {
            if chosen < property.weight {
                return property.schema.next(context);
            }
            chosen -= property.weight;
        }

        SchemaResult::Err("no streams available".into())
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
                properties,
            }))
        } else {
            Err(errors)
        }
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
                return Err(vec![visitor.schema_error("options must be specified")]);
            }
        };

        let option_specs: Vec<OptionSpec> = serde_json::from_value(options_value).map_err(|e| {
            vec![visitor.input_error("options", format!("options parsing failed: {e}"))]
        })?;

        if option_specs.is_empty() {
            return Err(vec![visitor.schema_error("options must not be empty")]);
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
