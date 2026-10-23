use super::{
    Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext, GeneratorResult,
};
use crate::build::BuildError;
use crate::effect::generator::{GeneratorEdge, Metadata};
use crate::parse::{GeneratorParseVisitor, GeneratorParser};
use crate::spec::{self, ParseError as Error};
use indexmap::IndexMap;
use serde_json::Map;

#[derive(Debug)]
pub struct Object {
    properties: IndexMap<String, Box<dyn Generator>>,
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

impl Generator for Object {
    fn next(&mut self, context: &GeneratorContext) -> GeneratorResult {
        let mut map = Map::new();
        let mut complete = true;
        let mut metadata: Vec<Metadata> = Vec::new();

        for (key, generator) in &mut self.properties {
            let result = generator.next(context);

            if let Some(value) = result.value {
                map.insert(key.clone(), value);
            } else {
                complete = false;
            }

            for mut result_metadata in result.metadata {
                result_metadata.prefix_attribute(key.clone().into());
                metadata.push(result_metadata)
            }
        }

        let value = if complete { Some(map.into()) } else { None };

        GeneratorResult { value, metadata }
    }
}

#[derive(Debug)]
pub struct ObjectBuilder {
    property_builders: IndexMap<String, Box<dyn GeneratorBuilder>>,
}

impl ObjectBuilder {
    pub fn set_property(
        &mut self,
        key: &str,
        builder: impl GeneratorBuilder + 'static,
    ) -> &mut Self {
        self.property_builders.insert(key.into(), Box::new(builder));
        self
    }

    pub fn property(mut self, key: &str, builder: impl GeneratorBuilder + 'static) -> Self {
        self.set_property(key, builder);
        self
    }
}

impl GeneratorBuilder for ObjectBuilder {
    fn build(&self, visitor: GeneratorBuildVisitor) -> Result<Box<dyn Generator>, Vec<BuildError>> {
        let mut errors = vec![];
        let mut properties = IndexMap::new();

        for (key, builder) in &self.property_builders {
            let property_visitor = visitor.follow_edge(GeneratorEdge {
                kind: "property",
                key: key.clone(),
            });
            match builder.build(property_visitor) {
                Ok(generator) => {
                    properties.insert(key.into(), generator);
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

impl GeneratorParser for ObjectParser {
    fn key(&self) -> &str {
        "object"
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<Error>> {
        if let Some(input) = visitor.spec().fields.get("properties") {
            serde_json::from_value::<IndexMap<String, spec::Generator>>(input.clone())
                .map_err(|err| {
                    vec![
                        visitor
                            .input_error("properties", format!("properties parsing failed: {err}")),
                    ]
                })
                .and_then(|properties| {
                    let mut errors = vec![];
                    let mut builder = Object::builder();

                    for (key, generator) in properties.into_iter() {
                        match visitor.parse_input_generator(
                            vec!["properties".into(), key.clone()],
                            generator,
                        ) {
                            Ok(stream) => {
                                builder.set_property(&key, stream);
                            }
                            Err(mut e) => errors.append(&mut e),
                        };
                    }

                    if errors.is_empty() {
                        Ok(Box::new(builder) as Box<dyn GeneratorBuilder>)
                    } else {
                        Err(errors)
                    }
                })
        } else {
            Err(vec![visitor.input_error("properties", "not specified")])
        }
    }
}
