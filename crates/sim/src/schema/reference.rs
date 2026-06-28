use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult};
use crate::build::BuildError;
use crate::log::{LogIndex, LogIndexConfig};
use crate::spec::SpecError as Error;
use crate::spec::{SchemaParseVisitor, SchemaParser};

#[derive(Debug)]
pub struct Reference {
    index: Box<dyn LogIndex>,
}

impl Reference {
    pub fn builder() -> ReferenceBuilder {
        ReferenceBuilder { config: None }
    }

    pub fn parser() -> ReferenceParser {
        ReferenceParser {}
    }
}

impl Schema for Reference {
    fn next(&mut self, _context: &SchemaContext) -> SchemaResult {
        match self.index.sample() {
            Some(effect_event) => SchemaResult::Ok {
                value: effect_event.value.clone(),
            },
            None => SchemaResult::Err("no event".into()),
        }
    }
}

#[derive(Debug)]
pub struct ReferenceBuilder {
    config: Option<LogIndexConfig>,
}

impl ReferenceBuilder {
    pub fn effect(mut self, effect: impl Into<String>) -> Self {
        self.set_effect(effect);
        self
    }

    pub fn set_effect(&mut self, effect: impl Into<String>) -> &mut Self {
        self.config = Some(LogIndexConfig::ByEffect {
            key: effect.into(),
            last_only: false,
        });
        self
    }
}

impl SchemaBuilder for ReferenceBuilder {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
        if let Some(config) = &self.config {
            Ok(Box::new(Reference {
                index: visitor.event_log.index(config.clone()),
            }))
        } else {
            Err(vec![visitor.error("config was not set")])
        }
    }
}

pub struct ReferenceParser {}

impl SchemaParser for ReferenceParser {
    fn should_parse(&self, visitor: &SchemaParseVisitor) -> bool {
        visitor.spec().stype == Some("reference".into())
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
        let mut builder = Reference::builder();

        let effect_key = match visitor.spec().fields.get("effect") {
            Some(k) if k.is_string() => k.as_str().unwrap().to_string(),
            Some(_) => {
                return Err(vec![Error {
                    path: Some(visitor.absolute_path()),
                    message: "effect must be a string".into(),
                }]);
            }
            None => {
                return Err(vec![Error {
                    path: Some(visitor.absolute_path()),
                    message: "effect must be specified".into(),
                }]);
            }
        };

        builder.set_effect(effect_key);

        Ok(Box::new(builder))
    }
}
