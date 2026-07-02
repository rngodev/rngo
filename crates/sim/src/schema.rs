pub(crate) mod array;
pub(crate) mod constant;
pub(crate) mod context;
pub(crate) mod function;
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
pub use number::Number;
pub use object::Object;
pub use reference::Reference;
pub use select::Select;
pub use string::Str;

use crate::build::{BuildError, SchemaEdge};
use crate::effect::TriggerEvent;
use crate::log::LogReader;
use rand_pcg::Pcg32;
use rand_seeder::Seeder;
use serde_json::Value;
use std::rc::Rc;

pub trait Schema: std::fmt::Debug {
    fn next(&mut self, context: &SchemaContext) -> SchemaResult;
}

pub struct SchemaContext<'a> {
    pub trigger: &'a TriggerEvent,
    pub simulation_start: DateTime<FixedOffset>,
    pub simulation_end: DateTime<FixedOffset>,
}

pub enum SchemaResult {
    Ok { value: Value },
    Err(String),
}

pub trait SchemaBuilder: std::fmt::Debug {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>>;
}

impl SchemaBuilder for Box<dyn SchemaBuilder> {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
        self.as_ref().build(visitor)
    }
}

pub struct SchemaBuildVisitor {
    pub event_log: Rc<dyn LogReader>,
    pub simulation_seed: u64,
    pub effect_key: String,
    pub path: Vec<SchemaEdge>,
}

impl Clone for SchemaBuildVisitor {
    fn clone(&self) -> Self {
        SchemaBuildVisitor {
            event_log: Rc::clone(&self.event_log),
            simulation_seed: self.simulation_seed,
            effect_key: self.effect_key.clone(),
            path: self.path.clone(),
        }
    }
}

impl SchemaBuildVisitor {
    pub fn rng(&self) -> Pcg32 {
        let path_str: String = self
            .path
            .iter()
            .map(|e| format!("{}{}", e.kind, e.key))
            .collect();
        let hash = format!("{}-{}-{}", self.simulation_seed, self.effect_key, path_str);
        Seeder::from(&hash).into_rng()
    }

    pub fn follow_edge(&self, edge: SchemaEdge) -> SchemaBuildVisitor {
        let mut new_self = self.clone();
        new_self.path.push(edge);
        new_self
    }

    pub fn error(&self, message: impl Into<String>) -> BuildError {
        BuildError::Schema {
            effect: self.effect_key.clone(),
            path: self.path.clone(),
            message: message.into(),
        }
    }
}
