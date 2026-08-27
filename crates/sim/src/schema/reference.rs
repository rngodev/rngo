use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult};
use crate::build::BuildError;
use crate::parse::{SchemaParseVisitor, SchemaParser};
use crate::run_log::{Cursor, RunLogIndex, RunLogIndexConfig};
use crate::schema::Metadata;
use crate::spec::ParseError as Error;

#[derive(Debug)]
pub struct Reference {
    index: Box<dyn RunLogIndex>,
}

impl Reference {
    pub fn builder() -> ReferenceBuilder {
        ReferenceBuilder {
            effect: None,
            cursor: Cursor::Random,
        }
    }

    pub fn parser() -> ReferenceParser {
        ReferenceParser {}
    }
}

impl Schema for Reference {
    fn next(&mut self, _context: &SchemaContext) -> SchemaResult {
        match self.index.sample() {
            Some(input_event) => SchemaResult {
                value: Some(input_event.data.clone()),
                metadata: input_event.metadata.clone(),
            },
            None => SchemaResult {
                value: None,
                metadata: vec![Metadata {
                    mtype: "skipped".into(),
                    attribute: None,
                    data: None,
                }],
            },
        }
    }
}

#[derive(Debug)]
pub struct ReferenceBuilder {
    effect: Option<String>,
    cursor: Cursor,
}

impl ReferenceBuilder {
    pub fn effect(mut self, effect: impl Into<String>) -> Self {
        self.set_effect(effect);
        self
    }

    pub fn set_effect(&mut self, effect: impl Into<String>) -> &mut Self {
        self.effect = Some(effect.into());
        self
    }

    pub fn cursor(mut self, cursor: Cursor) -> Self {
        self.set_cursor(cursor);
        self
    }

    pub fn set_cursor(&mut self, cursor: Cursor) -> &mut Self {
        self.cursor = cursor;
        self
    }
}

impl SchemaBuilder for ReferenceBuilder {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
        if let Some(key) = &self.effect {
            Ok(Box::new(Reference {
                index: visitor.event_run_log.index(RunLogIndexConfig::ByEffect {
                    key: key.clone(),
                    cursor: self.cursor,
                }),
            }))
        } else {
            Err(vec![visitor.error("config was not set")])
        }
    }
}

pub struct ReferenceParser {}

impl SchemaParser for ReferenceParser {
    fn key(&self) -> &str {
        "reference"
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
        let mut builder = Reference::builder();
        let mut errors = vec![];

        match visitor.spec().fields.get("effect") {
            Some(k) if k.is_string() => {
                builder.set_effect(k.as_str().unwrap().to_string());
            }
            Some(_) => errors.push(visitor.schema_error("effect must be a string")),
            None => errors.push(visitor.schema_error("effect must be specified")),
        }

        if let Some(v) = visitor.spec().fields.get("cursor") {
            match v.as_str() {
                Some("random") => {
                    builder.set_cursor(Cursor::Random);
                }
                Some("unique") => {
                    builder.set_cursor(Cursor::Unique);
                }
                _ => errors.push(
                    visitor.input_error("cursor", "cursor must be either \"random\" or \"unique\""),
                ),
            }
        }

        if errors.is_empty() {
            Ok(Box::new(builder))
        } else {
            Err(errors)
        }
    }
}
