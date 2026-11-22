use super::{Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorResult};
use crate::effect::generator::{
    GeneratorContext, GeneratorParseVisitor, GeneratorParser, Metadata,
};
use crate::error::{BuildErrors, ParseErrors};
use crate::moment::offset_timestamp;
use jiff::SignedDuration;

#[derive(Debug)]
pub(crate) struct Context {
    path: ContextPath,
}

impl Context {
    pub(crate) fn builder() -> ContextBuilder {
        ContextBuilder { path: None }
    }

    pub(crate) fn parser() -> ContextParser {
        ContextParser {}
    }
}

impl Generator for Context {
    fn next(&mut self, context: &GeneratorContext) -> GeneratorResult {
        match self.path {
            ContextPath::TriggerEvent => match &context.trigger.input_event {
                Some(input_event) => match serde_json::to_value(input_event.as_ref()) {
                    Ok(value) => GeneratorResult {
                        value: Some(value),
                        metadata: vec![],
                    },
                    Err(e) => error_result(e.to_string()),
                },
                None => error_result("no value for trigger"),
            },
            ContextPath::SimOffset => GeneratorResult {
                value: Some((context.trigger.sim_offset as f64 / 1000.0).into()),
                metadata: vec![],
            },
            ContextPath::SimStart => GeneratorResult {
                value: Some(context.origin.to_string().into()),
                metadata: vec![],
            },
            ContextPath::ClockNow => {
                let now = offset_timestamp(
                    context.origin,
                    SignedDuration::from_millis(context.trigger.sim_offset),
                );
                GeneratorResult {
                    value: Some(now.to_string().into()),
                    metadata: vec![],
                }
            }
        }
    }
}

fn error_result(message: impl Into<String>) -> GeneratorResult {
    GeneratorResult {
        value: None,
        metadata: vec![
            Metadata::new("error").data(serde_json::json!({ "message": message.into() })),
        ],
    }
}

#[derive(Debug)]
enum ContextPath {
    TriggerEvent,
    SimOffset,
    SimStart,
    ClockNow,
}

impl TryFrom<Vec<String>> for ContextPath {
    type Error = String;

    fn try_from(value: Vec<String>) -> Result<Self, Self::Error> {
        let strs: Vec<&str> = value.iter().map(String::as_str).collect();
        match strs.as_slice() {
            ["trigger", "event"] => Ok(ContextPath::TriggerEvent),
            ["sim", "offset"] => Ok(ContextPath::SimOffset),
            ["sim", "start"] => Ok(ContextPath::SimStart),
            ["clock", "now"] => Ok(ContextPath::ClockNow),
            _ => Err(format!("unknown path: {:?}", value)),
        }
    }
}

#[derive(Debug)]
pub struct ContextBuilder {
    path: Option<Vec<String>>,
}

impl ContextBuilder {
    pub fn path(mut self, path: impl IntoIterator<Item: Into<String>>) -> Self {
        self.path = Some(path.into_iter().map(Into::into).collect());
        self
    }
}

impl GeneratorBuilder for ContextBuilder {
    fn build(
        self: Box<Self>,
        visitor: GeneratorBuildVisitor,
    ) -> Result<Box<dyn Generator>, BuildErrors> {
        let path = match self.path {
            Some(p) => p,
            None => return Err(visitor.error("path was not set").into()),
        };

        ContextPath::try_from(path)
            .map(|p| Box::new(Context { path: p }) as Box<dyn Generator>)
            .map_err(|e| visitor.error(format!("invalid context path: {e}")).into())
    }
}

pub(crate) struct ContextParser {}

impl GeneratorParser for ContextParser {
    fn key(&self) -> &str {
        "context"
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, ParseErrors> {
        let path = match visitor.spec().fields.get("path") {
            Some(v) => serde_json::from_value::<Vec<String>>(v.clone()).map_err(|e| {
                vec![visitor.field_error("path", format!("path parsing failed: {e}"))]
            })?,
            None => {
                return Err(visitor.error("path must be specified").into());
            }
        };

        let mut builder = Context::builder();
        builder = builder.path(path);
        Ok(Box::new(builder))
    }
}
