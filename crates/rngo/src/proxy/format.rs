pub(crate) mod sql;
pub(crate) mod template;

use crate::effect::Input;
use crate::{BuildErrors, FormatError};
use std::fmt::Debug;

mod parser;
pub use parser::{FormatParseContext, FormatParser};
pub(crate) use sql::SqlFormat;
pub(crate) use template::TemplateFormat;

/// Turns an input into the string a channel delivers.
pub trait Format: Debug + Send + Sync {
    fn format(&self, event: &Input) -> Result<String, FormatError>;
}

impl<F: Format + ?Sized> Format for std::sync::Arc<F> {
    fn format(&self, event: &Input) -> Result<String, FormatError> {
        self.as_ref().format(event)
    }
}

/// Builds a [`Format`] when its channel is built; `channel_key` names the channel in build errors.
/// Every `Format` is its own builder.
pub trait FormatBuilder: Debug + Send {
    fn build(self: Box<Self>, channel_key: &str) -> Result<Box<dyn Format>, BuildErrors>;
}

impl FormatBuilder for Box<dyn FormatBuilder> {
    fn build(self: Box<Self>, channel_key: &str) -> Result<Box<dyn Format>, BuildErrors> {
        (*self).build(channel_key)
    }
}

impl<F: Format + 'static> FormatBuilder for F {
    fn build(self: Box<Self>, _channel_key: &str) -> Result<Box<dyn Format>, BuildErrors> {
        Ok(self)
    }
}
