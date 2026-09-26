mod pool;
mod simple;
mod sqlite;

use crate::Output;
use crate::effect::Input;
use rand_pcg::Pcg32;
use serde_json::Value;
use std::rc::Rc;

pub use simple::SimpleEventRunLog;
pub use sqlite::SqliteRunLog;

pub trait RunLogWriter: std::fmt::Debug {
    fn push_input(&self, input: Input);
    fn push_output(&self, output: Output);
    fn push_metadata(&self, metadata: Metadata);
}

pub trait RunLogReader: std::fmt::Debug {
    fn last(&self) -> Option<Rc<Input>>;
    fn last_for_effect(&self, key: &str) -> Option<Rc<Input>>;
    fn random_for_effect(&self, key: &str, rng: &mut Pcg32) -> Option<Rc<Input>>;
    fn unique_for_effect(&self, key: &str, cursor: &str, rng: &mut Pcg32) -> Option<Rc<Input>>;
    fn query(&self, query: &str) -> Option<Value>;
}

#[derive(Clone, Debug)]
pub struct Metadata {
    pub mtype: String,
    pub input_id: Option<i64>,
    pub output_id: Option<i64>,
    pub offset: Option<u64>,
    pub data: Option<Value>,
    pub segment: Option<String>,
}
