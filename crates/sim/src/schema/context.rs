use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaResult};
use crate::build::BuildError;
use crate::parse::{SchemaParseVisitor, SchemaParser};
use crate::schema::{Metadata, SchemaContext};
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

impl Schema for Context {
    fn next(&mut self, context: &SchemaContext) -> SchemaResult {
        match self.path {
            ContextPath::TriggerEvent => match &context.trigger.input_event {
                Some(input_event) => match serde_json::to_value(input_event.as_ref()) {
                    Ok(value) => SchemaResult {
                        value: Some(value),
                        metadata: vec![],
                    },
                    Err(e) => error_result(e.to_string()),
                },
                None => error_result("no value for trigger"),
            },
            ContextPath::SimOffset => SchemaResult {
                value: Some(context.trigger.sim_offset.into()),
                metadata: vec![],
            },
            ContextPath::SimStart => SchemaResult {
                value: Some(context.simulation_start.to_rfc3339().into()),
                metadata: vec![],
            },
            ContextPath::SimEnd => SchemaResult {
                value: Some(context.simulation_end.to_rfc3339().into()),
                metadata: vec![],
            },
            ContextPath::ClockNow => {
                let now = context.simulation_start
                    + chrono::Duration::seconds(context.trigger.sim_offset as i64);
                SchemaResult {
                    value: Some(now.to_rfc3339().into()),
                    metadata: vec![],
                }
            }
        }
    }
}

fn error_result(message: impl Into<String>) -> SchemaResult {
    SchemaResult {
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

impl SchemaBuilder for ContextBuilder {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
        let path = match &self.path {
            Some(p) => p.clone(),
            None => return Err(vec![visitor.error("path was not set")]),
        };

        ContextPath::try_from(path)
            .map(|p| Box::new(Context { path: p }) as Box<dyn Schema>)
            .map_err(|e| vec![visitor.error(format!("invalid context path: {e}"))])
    }
}

pub struct ContextParser {}

impl SchemaParser for ContextParser {
    fn key(&self) -> &str {
        "context"
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
        let path = match visitor.spec().fields.get("path") {
            Some(v) => serde_json::from_value::<Vec<String>>(v.clone()).map_err(|e| {
                vec![visitor.input_error("path", format!("path parsing failed: {e}"))]
            })?,
            None => {
                return Err(vec![visitor.schema_error("path must be specified")]);
            }
        };

        let mut builder = Context::builder();
        builder.set_path(path);
        Ok(Box::new(builder))
    }
}
