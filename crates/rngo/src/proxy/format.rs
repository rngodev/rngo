mod sql;
mod template;

use crate::FormatError;
use crate::effect::Input;
use std::fmt::Debug;

mod parser;
pub use parser::{FormatParseContext, FormatParser};
pub use sql::SqlFormat;
pub use template::TemplateFormat;

/// Turns an input into the string a channel delivers.
pub trait Format: Debug {
    fn format(&self, event: &Input) -> Result<String, FormatError>;
}

impl<F: Format + ?Sized> Format for std::rc::Rc<F> {
    fn format(&self, event: &Input) -> Result<String, FormatError> {
        self.as_ref().format(event)
    }
}
