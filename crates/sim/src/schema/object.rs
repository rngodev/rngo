use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult};
use crate::build::{BuildError, SchemaEdge};
use crate::parse::{SchemaParseVisitor, SchemaParser};
use crate::spec::{self, ParseError as Error};
use indexmap::IndexMap;
use serde_json::Map;

#[derive(Debug)]
pub struct Object {
    properties: IndexMap<String, Box<dyn Schema>>,
}

impl Object {
    pub fn builder() -> ObjectBuilder {
        ObjectBuilder {
            property_builders: IndexMap::new(),
        }
    }

    pub fn parser() -> ObjectParser {
        ObjectParser {}
    }
}

impl Schema for Object {
    fn next(&mut self, context: &SchemaContext) -> SchemaResult {
        let mut map = Map::new();

        for (key, schema) in &mut self.properties {
            match schema.next(context) {
                SchemaResult::Ok { value } => {
                    map.insert(key.clone(), value);
                }
                SchemaResult::Err(e) => return SchemaResult::Err(e),
            }
        }

        SchemaResult::Ok { value: map.into() }
    }
}

#[derive(Debug)]
pub struct ObjectBuilder {
    property_builders: IndexMap<String, Box<dyn SchemaBuilder>>,
}

impl ObjectBuilder {
    pub fn set_property(&mut self, key: &str, builder: impl SchemaBuilder + 'static) -> &mut Self {
        self.property_builders.insert(key.into(), Box::new(builder));
        self
    }

    pub fn property(mut self, key: &str, builder: impl SchemaBuilder + 'static) -> Self {
        self.set_property(key, builder);
        self
    }
}

impl SchemaBuilder for ObjectBuilder {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
        let mut errors = vec![];
        let mut properties = IndexMap::new();

        for (key, builder) in &self.property_builders {
            let property_visitor = visitor.follow_edge(SchemaEdge {
                kind: "property",
                key: key.clone(),
            });
            match builder.build(property_visitor) {
                Ok(schema) => {
                    properties.insert(key.into(), schema);
                }
                Err(mut e) => errors.append(&mut e),
            }
        }

        if errors.is_empty() {
            Ok(Box::new(Object { properties }))
        } else {
            Err(errors)
        }
    }
}

pub struct ObjectParser {}

impl SchemaParser for ObjectParser {
    fn key(&self) -> &str {
        "object"
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
        if let Some(input) = visitor.spec().fields.get("properties") {
            serde_json::from_value::<IndexMap<String, spec::Schema>>(input.clone())
                .map_err(|err| {
                    vec![
                        visitor
                            .input_error("properties", format!("properties parsing failed: {err}")),
                    ]
                })
                .and_then(|properties| {
                    let mut errors = vec![];
                    let mut builder = Object::builder();

                    for (key, schema) in properties.into_iter() {
                        match visitor
                            .parse_input_schema(vec!["properties".into(), key.clone()], schema)
                        {
                            Ok(stream) => {
                                builder.set_property(&key, stream);
                            }
                            Err(mut e) => errors.append(&mut e),
                        };
                    }

                    if errors.is_empty() {
                        Ok(Box::new(builder) as Box<dyn SchemaBuilder>)
                    } else {
                        Err(errors)
                    }
                })
        } else {
            Err(vec![visitor.input_error("properties", "not specified")])
        }
    }
}
