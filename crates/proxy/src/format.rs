mod sql;

use rngo_core::Input;
use std::fmt::Debug;

pub use sql::SqlFormat;

pub trait Format: Debug {
    fn format(&self, event: &Input) -> Result<String, String>;
}
