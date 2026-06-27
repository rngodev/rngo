use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaResult};
use crate::build::BuildError;
use crate::schema::SchemaContext;
use crate::spec::SpecError as Error;
use crate::spec::{SchemaParseVisitor, SchemaParser};

#[derive(Debug)]
enum ContextPath {
    Event,
    Offset,
}

#[derive(Debug)]
pub struct Context {
    path: ContextPath,
}

impl Context {
    pub fn new(path: Vec<String>) -> Result<Self, String> {
        let strs: Vec<&str> = path.iter().map(String::as_str).collect();
        let path = match strs.as_slice() {
            ["trigger", "event"] => ContextPath::Event,
            ["trigger", "offset"] => ContextPath::Offset,
            _ => return Err(format!("unknown path: {:?}", path)),
        };
        Ok(Self { path })
    }

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
            ContextPath::Event => match &context.trigger.effect_event {
                Some(effect_event) => SchemaResult::Ok {
                    value: effect_event.value.clone(),
                },
                None => SchemaResult::Err("no value for trigger".into()),
            },
            ContextPath::Offset => SchemaResult::Ok {
                value: context.trigger.offset.into(),
            },
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

        Context::new(path)
            .map(|c| Box::new(c) as Box<dyn Schema>)
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
