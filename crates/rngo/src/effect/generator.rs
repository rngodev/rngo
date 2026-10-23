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

pub use array::Array;
use chrono::{DateTime, FixedOffset};
pub use constant::Constant;
pub use context::Context;
pub use function::Function;
pub use json_pointer::{JsonPointer, JsonPointerPart};
pub use number::Number;
pub use object::Object;
pub use reference::Reference;
pub use select::Select;
pub use string::Str;

use crate::build::BuildError;
use crate::effect::TriggerEvent;
use crate::log::RunLogReader;
use rand_pcg::Pcg32;
use rand_seeder::Seeder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::rc::Rc;

pub trait Generator: std::fmt::Debug {
    fn next(&mut self, context: &GeneratorContext) -> GeneratorResult;
}

/// One step down the generator tree: `kind` is the kind of step (e.g. a property or an array item) and `key` names it.
#[derive(Debug, Clone)]
pub struct GeneratorEdge {
    pub kind: &'static str,
    pub key: String,
}

pub struct GeneratorContext<'a> {
    pub trigger: &'a TriggerEvent,
    pub simulation_start: DateTime<FixedOffset>,
    pub simulation_end: DateTime<FixedOffset>,
}

pub struct GeneratorResult {
    pub(crate) value: Option<Value>,
    pub(crate) metadata: Vec<Metadata>,
}

impl From<Value> for GeneratorResult {
    fn from(value: Value) -> Self {
        Self {
            value: Some(value),
            metadata: Vec::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(rename = "type")]
    pub mtype: String,
    pub attribute: Option<JsonPointer>,
    pub data: Option<Value>,
}

impl Metadata {
    fn prefix_attribute(&mut self, part: JsonPointerPart) {
        if let Some(attribute) = &mut self.attribute {
            attribute.prefix(part)
        } else {
            self.attribute = Some(part.into())
        }
    }
}

pub trait GeneratorBuilder: std::fmt::Debug {
    fn build(&self, visitor: GeneratorBuildVisitor) -> Result<Box<dyn Generator>, Vec<BuildError>>;
}

impl GeneratorBuilder for Box<dyn GeneratorBuilder> {
    fn build(&self, visitor: GeneratorBuildVisitor) -> Result<Box<dyn Generator>, Vec<BuildError>> {
        self.as_ref().build(visitor)
    }
}

pub struct GeneratorBuildVisitor {
    pub event_run_log: Rc<dyn RunLogReader>,
    pub simulation_seed: u64,
    pub effect_key: String,
    pub path: Vec<GeneratorEdge>,
}

impl Clone for GeneratorBuildVisitor {
    fn clone(&self) -> Self {
        GeneratorBuildVisitor {
            event_run_log: Rc::clone(&self.event_run_log),
            simulation_seed: self.simulation_seed,
            effect_key: self.effect_key.clone(),
            path: self.path.clone(),
        }
    }
}

impl GeneratorBuildVisitor {
    pub fn rng(&self) -> Pcg32 {
        let hash = format!("{}-{}", self.simulation_seed, self.path_id());
        Seeder::from(&hash).into_rng()
    }

    pub fn path_id(&self) -> String {
        let path_str: String = self
            .path
            .iter()
            .map(|e| format!("{}{}", e.kind, e.key))
            .collect();
        format!("{}-{}", self.effect_key, path_str)
    }

    pub fn follow_edge(&self, edge: GeneratorEdge) -> GeneratorBuildVisitor {
        let mut new_self = self.clone();
        new_self.path.push(edge);
        new_self
    }

    pub fn error(&self, message: impl Into<String>) -> BuildError {
        BuildError::generator(self.effect_key.clone(), self.path.clone(), message.into())
    }
}
