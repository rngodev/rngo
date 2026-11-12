mod sql;
mod template;

use crate::FormatError;
use crate::effect::Input;
use std::fmt::Debug;

pub use sql::SqlFormat;
pub use template::TemplateFormat;

/// Turns an input into the string a channel delivers.
pub trait Format: Debug {
    fn format(&self, event: &Input) -> Result<String, FormatError>;
}
