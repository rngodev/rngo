pub(crate) mod array;
pub(crate) mod constant;
pub(crate) mod context;
pub(crate) mod custom;
pub(crate) mod function;
mod json_pointer;
pub(crate) mod number;
pub(crate) mod object;
pub(crate) mod reference;
pub(crate) mod select;
pub(crate) mod string;

use crate::error::BuildErrors;
pub(crate) use array::Array;
pub(crate) use constant::Constant;
pub(crate) use context::Context;
pub(crate) use function::Function;
use jiff::Timestamp;
pub use json_pointer::{JsonPointer, JsonPointerPart};
pub(crate) use number::Number;
pub(crate) use object::Object;
pub(crate) use reference::Reference;
pub(crate) use select::Select;
pub(crate) use string::Str;

pub use crate::effect::TriggerEvent;
use crate::error::BuildError;
use crate::log::RunLogReader;
mod parser;
pub use parser::{GeneratorParseVisitor, GeneratorParser};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub trait Generator: std::fmt::Debug + Send {
    fn next(&mut self, context: &GeneratorContext) -> GeneratorResult;
}

/// One step down the generator tree: `kind` is the kind of step (e.g. a property or an array item) and `key` names it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GeneratorEdge {
    pub kind: GeneratorEdgeKind,
    pub key: String,
}

impl GeneratorEdge {
    pub fn new(kind: GeneratorEdgeKind, key: impl Into<String>) -> Self {
        GeneratorEdge {
            kind,
            key: key.into(),
        }
    }

    /// The spec fields this step descends through, e.g. `properties.<key>` for a property.
    pub fn spec_path(&self) -> Vec<String> {
        match self.kind {
            GeneratorEdgeKind::Property => vec!["properties".into(), self.key.clone()],
            GeneratorEdgeKind::Variable => vec!["variables".into(), self.key.clone()],
            GeneratorEdgeKind::Option => {
                vec!["options".into(), self.key.clone(), "generator".into()]
            }
            GeneratorEdgeKind::Items => vec!["items".into()],
            GeneratorEdgeKind::Other(kind) => vec![kind.into(), self.key.clone()],
        }
    }
}

/// The kind of step a `GeneratorEdge` takes down the generator tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeneratorEdgeKind {
    /// An `object` property.
    Property,
    /// A `function` variable.
    Variable,
    /// A `select` option.
    Option,
    /// An `array`'s items.
    Items,
    /// A step defined by a custom generator; the name is also the spec field it descends into.
    Other(&'static str),
}

impl GeneratorEdgeKind {
    /// The kind's name, e.g. `property`; part of the generator's seeded path.
    pub fn as_str(&self) -> &'static str {
        match self {
            GeneratorEdgeKind::Property => "property",
            GeneratorEdgeKind::Variable => "variable",
            GeneratorEdgeKind::Option => "option",
            GeneratorEdgeKind::Items => "items",
            GeneratorEdgeKind::Other(kind) => kind,
        }
    }
}

/// What a generator sees each time its effect fires.
pub struct GeneratorContext<'a> {
    pub(crate) trigger: &'a TriggerEvent,
    pub(crate) origin: Timestamp,
}

impl GeneratorContext<'_> {
    /// The trigger firing that this value is being generated for.
    pub fn trigger(&self) -> &TriggerEvent {
        self.trigger
    }

    /// The instant sim offsets are measured from.
    pub fn origin(&self) -> Timestamp {
        self.origin
    }
}

/// What a generator produced: a value, or none to skip the attempt, plus any metadata.
pub struct GeneratorResult {
    pub(crate) value: Option<Value>,
    pub(crate) metadata: Vec<GeneratorMetadata>,
}

impl GeneratorResult {
    /// No value, so the effect skips this attempt.
    pub fn skip() -> Self {
        Self {
            value: None,
            metadata: Vec::default(),
        }
    }

    pub fn metadata(mut self, metadata: Vec<GeneratorMetadata>) -> Self {
        self.metadata = metadata;
        self
    }
}

impl From<Value> for GeneratorResult {
    fn from(value: Value) -> Self {
        Self {
            value: Some(value),
            metadata: Vec::default(),
        }
    }
}

/// A note a generator attaches to its result, such as an error; `attribute` points at the part of
/// the value it concerns.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GeneratorMetadata {
    #[serde(rename = "type")]
    pub kind: String,
    pub attribute: Option<JsonPointer>,
    pub data: Option<Value>,
}

impl GeneratorMetadata {
    /// GeneratorMetadata of type `kind`, about the whole value and with no data.
    pub fn new(kind: impl Into<String>) -> Self {
        GeneratorMetadata {
            kind: kind.into(),
            attribute: None,
            data: None,
        }
    }

    /// Points it at part of the value.
    pub fn attribute(mut self, attribute: JsonPointer) -> Self {
        self.attribute = Some(attribute);
        self
    }

    pub fn data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    fn prefix_attribute(&mut self, part: JsonPointerPart) {
        if let Some(attribute) = &mut self.attribute {
            attribute.prefix(part)
        } else {
            self.attribute = Some(part.into())
        }
    }
}

pub trait GeneratorBuilder: std::fmt::Debug + Send {
    fn build(
        self: Box<Self>,
        visitor: GeneratorBuildVisitor,
    ) -> Result<Box<dyn Generator>, BuildErrors>;
}

impl GeneratorBuilder for Box<dyn GeneratorBuilder> {
    fn build(
        self: Box<Self>,
        visitor: GeneratorBuildVisitor,
    ) -> Result<Box<dyn Generator>, BuildErrors> {
        (*self).build(visitor)
    }
}

/// What a generator builder gets when the simulation builds: the run log, the effect, and where in
/// the generator tree it sits.
pub struct GeneratorBuildVisitor {
    pub(crate) event_run_log: Arc<dyn RunLogReader>,
    pub(crate) simulation_seed: u64,
    pub(crate) effect_key: String,
    pub(crate) path: Vec<GeneratorEdge>,
    pub(crate) spec_path: Vec<String>,
}

impl Clone for GeneratorBuildVisitor {
    fn clone(&self) -> Self {
        GeneratorBuildVisitor {
            event_run_log: Arc::clone(&self.event_run_log),
            simulation_seed: self.simulation_seed,
            effect_key: self.effect_key.clone(),
            path: self.path.clone(),
            spec_path: self.spec_path.clone(),
        }
    }
}

impl GeneratorBuildVisitor {
    /// The run log, for generators that look up earlier inputs.
    pub fn run_log(&self) -> &Arc<dyn RunLogReader> {
        &self.event_run_log
    }

    /// The key of the effect being built.
    pub fn effect_key(&self) -> &str {
        &self.effect_key
    }

    /// The edges from the effect's root generator down to this one.
    pub fn path(&self) -> &[GeneratorEdge] {
        &self.path
    }

    /// An RNG seeded from the simulation seed and this generator's path, so each generator draws its own stream.
    pub fn rng(&self) -> crate::Rng {
        let hash = format!("{}-{}", self.simulation_seed, self.path_id());
        crate::Rng::from_seed(&hash)
    }

    pub fn path_id(&self) -> String {
        let path_str: String = self
            .path
            .iter()
            .map(|e| format!("{}{}", e.kind.as_str(), e.key))
            .collect();
        format!("{}-{}", self.effect_key, path_str)
    }

    pub fn follow_edge(&self, edge: GeneratorEdge) -> GeneratorBuildVisitor {
        let mut new_self = self.clone();
        new_self.spec_path.extend(edge.spec_path());
        new_self.path.push(edge);
        new_self
    }

    /// This visitor for the root generator of custom type `name`, whose spec lives at `generators.<name>.generator`.
    pub(crate) fn in_custom_type(&self, name: &str) -> GeneratorBuildVisitor {
        let mut new_self = self.clone();
        new_self.spec_path = vec!["generators".into(), name.into(), "generator".into()];
        new_self
    }

    /// An error located at this generator, by effect and path through the generator tree.
    pub fn error(&self, message: impl Into<String>) -> BuildError {
        BuildError::generator(self.effect_key.clone(), self.path.clone(), message.into())
            .at_spec_path(self.spec_path.clone())
    }
}
