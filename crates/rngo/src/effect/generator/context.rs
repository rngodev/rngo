use super::{Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorResult};
use crate::build::BuildError;
use crate::effect::generator::{GeneratorContext, Metadata};
use crate::parse::{GeneratorParseVisitor, GeneratorParser};
use crate::spec::ParseError as Error;
use serde_json;

#[derive(Debug)]
pub struct Context {
    path: ContextPath,
}

impl Context {
    pub fn builder() -> ContextBuilder {
        ContextBuilder { path: None }
    }

    pub fn parser() -> ContextParser {
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
                value: Some(context.simulation_start.to_rfc3339().into()),
                metadata: vec![],
            },
            ContextPath::SimEnd => GeneratorResult {
                value: Some(context.simulation_end.to_rfc3339().into()),
                metadata: vec![],
            },
            ContextPath::ClockNow => {
                let now = context.simulation_start
                    + chrono::Duration::milliseconds(context.trigger.sim_offset);
                GeneratorResult {
                    value: Some(now.to_rfc3339().into()),
                    metadata: vec![],
                }
            }
        }
    }
}

fn error_result(message: impl Into<String>) -> GeneratorResult {
    GeneratorResult {
        value: None,
        metadata: vec![Metadata {
            mtype: "error".into(),
            attribute: None,
            data: Some(serde_json::json!({ "message": message.into() })),
        }],
    }
}

#[derive(Debug)]
enum ContextPath {
    TriggerEvent,
    SimOffset,
    SimStart,
    SimEnd,
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
            ["sim", "end"] => Ok(ContextPath::SimEnd),
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
        self.set_path(path);
        self
    }

    pub fn set_path(&mut self, path: impl IntoIterator<Item: Into<String>>) -> &mut Self {
        self.path = Some(path.into_iter().map(Into::into).collect());
        self
    }
}

impl GeneratorBuilder for ContextBuilder {
    fn build(&self, visitor: GeneratorBuildVisitor) -> Result<Box<dyn Generator>, Vec<BuildError>> {
        let path = match &self.path {
            Some(p) => p.clone(),
            None => return Err(vec![visitor.error("path was not set")]),
        };

        ContextPath::try_from(path)
            .map(|p| Box::new(Context { path: p }) as Box<dyn Generator>)
            .map_err(|e| vec![visitor.error(format!("invalid context path: {e}"))])
    }
}

pub struct ContextParser {}

impl GeneratorParser for ContextParser {
    fn key(&self) -> &str {
        "context"
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<Error>> {
        let path = match visitor.spec().fields.get("path") {
            Some(v) => serde_json::from_value::<Vec<String>>(v.clone()).map_err(|e| {
                vec![visitor.input_error("path", format!("path parsing failed: {e}"))]
            })?,
            None => {
                return Err(vec![visitor.invalid("path must be specified")]);
            }
        };

        let mut builder = Context::builder();
        builder.set_path(path);
        Ok(Box::new(builder))
    }
}
