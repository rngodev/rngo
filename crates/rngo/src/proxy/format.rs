mod sql;
mod template;

use crate::effect::Input;
use std::fmt::Debug;

pub use sql::SqlFormat;
pub use template::TemplateFormat;

pub trait Format: Debug {
    fn format(&self, event: &Input) -> Result<String, String>;
}
