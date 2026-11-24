use super::{
    Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext, GeneratorResult,
};
use crate::Rng;
use crate::effect::generator::{GeneratorMetadata, GeneratorParseVisitor, GeneratorParser};
use crate::error::{BuildErrors, ParseErrors};
use crate::log::LogReader;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Random,
    Unique,
}

#[derive(Debug)]
pub(crate) struct Reference {
    log_reader: Arc<dyn LogReader>,
    key: String,
    cursor: Cursor,
    unique_cursor: Option<String>,
    rng: Rng,
}

impl Reference {
    pub(crate) fn builder() -> ReferenceBuilder {
        ReferenceBuilder {
            effect: None,
            cursor: Cursor::Random,
        }
    }

    pub(crate) fn parser() -> ReferenceParser {
        ReferenceParser {}
    }
}

impl Generator for Reference {
    fn next(&mut self, _context: &GeneratorContext) -> GeneratorResult {
        let sampled = match self.cursor {
            Cursor::Random => self.log_reader.random_for_effect(&self.key, &mut self.rng),
            Cursor::Unique => {
                let cursor = self
                    .unique_cursor
                    .as_deref()
                    .expect("unique_cursor set for Cursor::Unique");
                self.log_reader
                    .unique_for_effect(&self.key, cursor, &mut self.rng)
            }
        };

        match sampled {
            Some(input_event) => GeneratorResult {
                value: Some(input_event.data.clone()),
                metadata: input_event.metadata.clone(),
            },
            None => GeneratorResult {
                value: None,
                metadata: vec![GeneratorMetadata::new("skipped")],
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
        self.effect = Some(effect.into());
        self
    }

    pub fn cursor(mut self, cursor: Cursor) -> Self {
        self.cursor = cursor;
        self
    }
}

impl GeneratorBuilder for ReferenceBuilder {
    fn build(
        self: Box<Self>,
        visitor: GeneratorBuildVisitor,
    ) -> Result<Box<dyn Generator>, BuildErrors> {
        if let Some(key) = self.effect {
            let unique_cursor = matches!(self.cursor, Cursor::Unique).then(|| visitor.path_id());

            Ok(Box::new(Reference {
                log_reader: visitor.log_reader.clone(),
                key,
                cursor: self.cursor,
                unique_cursor,
                rng: visitor.rng(),
            }))
        } else {
            Err(visitor.error("config was not set").into())
        }
    }
}

pub(crate) struct ReferenceParser {}

impl GeneratorParser for ReferenceParser {
    fn key(&self) -> &str {
        "reference"
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, ParseErrors> {
        let mut builder = Reference::builder();
        let mut errors = vec![];

        match visitor.spec().fields.get("effect").map(|v| v.as_str()) {
            Some(Some(effect)) => {
                builder = builder.effect(effect.to_string());
            }
            Some(_) => errors.push(visitor.error("effect must be a string")),
            None => errors.push(visitor.error("effect must be specified")),
        }

        if let Some(v) = visitor.spec().fields.get("cursor") {
            match v.as_str() {
                Some("random") => {
                    builder = builder.cursor(Cursor::Random);
                }
                Some("unique") => {
                    builder = builder.cursor(Cursor::Unique);
                }
                _ => errors.push(
                    visitor.field_error("cursor", "cursor must be either \"random\" or \"unique\""),
                ),
            }
        }

        if errors.is_empty() {
            Ok(Box::new(builder))
        } else {
            Err(errors.into())
        }
    }
}
