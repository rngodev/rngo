use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaResult};
use crate::build::BuildError;
use crate::schema::SchemaContext;
use crate::spec::{SchemaParseVisitor, SchemaParser, SpecError as Error};
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
            ContextPath::TriggerEvent => match &context.trigger.effect_event {
                Some(effect_event) => match serde_json::to_value(effect_event.as_ref()) {
                    Ok(value) => SchemaResult::Ok { value },
                    Err(e) => SchemaResult::Err(e.to_string()),
                },
                None => SchemaResult::Err("no value for trigger".into()),
            },
            ContextPath::SimOffset => SchemaResult::Ok {
                value: context.trigger.sim_offset.into(),
            },
            ContextPath::SimStart => SchemaResult::Ok {
                value: context.simulation_start.to_rfc3339().into(),
            },
            ContextPath::SimEnd => SchemaResult::Ok {
                value: context.simulation_end.to_rfc3339().into(),
            },
            ContextPath::ClockNow => {
                let now = context.simulation_start
                    + chrono::Duration::seconds(context.trigger.sim_offset as i64);
                SchemaResult::Ok {
                    value: now.to_rfc3339().into(),
                }
            }
        }
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
            _ => return Err(format!("unknown path: {:?}", value)),
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
    fn should_parse(&self, visitor: &SchemaParseVisitor) -> bool {
        visitor.spec().stype == Some("context".into())
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
        let path = match visitor.spec().fields.get("path") {
            Some(v) => serde_json::from_value::<Vec<String>>(v.clone()).map_err(|e| {
                vec![Error {
                    path: Some(visitor.absolute_sub_path(vec!["path".into()])),
                    message: format!("path parsing failed: {e}"),
                }]
            })?,
            None => {
                return Err(vec![Error {
                    path: Some(visitor.absolute_path()),
                    message: "path must be specified".into(),
                }]);
            }
        };

        let mut builder = Context::builder();
        builder.set_path(path);
        Ok(Box::new(builder))
    }
}
