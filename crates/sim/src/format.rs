mod sql;

use serde_json::Value;
use std::fmt::Debug;

pub use sql::SqlFormat;

pub trait Format: Debug {
    fn format(&self, value: &Value) -> String;
}
