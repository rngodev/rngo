use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult};
use crate::build::BuildError;
use crate::parse::{SchemaParseVisitor, SchemaParser};
use crate::run_log::RunLogReader;
use crate::schema::Metadata;
use crate::spec::ParseError as Error;
use rand_pcg::Pcg32;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Random,
    Unique,
}

#[derive(Debug)]
pub struct Reference {
    event_run_log: Rc<dyn RunLogReader>,
    key: String,
    cursor: Cursor,
    unique_cursor: Option<String>,
    rng: Pcg32,
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
        let sampled = match self.cursor {
            Cursor::Random => self
                .event_run_log
                .random_for_effect(&self.key, &mut self.rng),
            Cursor::Unique => {
                let cursor = self
                    .unique_cursor
                    .as_deref()
                    .expect("unique_cursor set for Cursor::Unique");
                self.event_run_log
                    .unique_for_effect(&self.key, cursor, &mut self.rng)
            }
        };

        match sampled {
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
            let unique_cursor = matches!(self.cursor, Cursor::Unique).then(|| visitor.path_id());

            Ok(Box::new(Reference {
                event_run_log: visitor.event_run_log.clone(),
                key: key.clone(),
                cursor: self.cursor,
                unique_cursor,
                rng: visitor.rng(),
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
