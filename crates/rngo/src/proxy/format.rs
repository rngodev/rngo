pub(crate) mod csv;
pub(crate) mod sql;
pub(crate) mod template;

use crate::effect::Input;
use crate::{BuildErrors, FormatError};
use std::fmt::Debug;
use std::marker::PhantomData;

mod parser;
pub(crate) use csv::CsvFormat;
pub use parser::{FormatParseContext, FormatParser};
pub(crate) use sql::SqlFormat;
pub(crate) use template::TemplateFormat;

/// Turns an input into the string a channel delivers.
pub trait Format: Debug + Send + Sync {
    fn format(&self, input: &Input, context: &FormatContext) -> Result<String, FormatError>;
}

/// What a format sees when it formats an input. A channel gets one from
/// [`SendContext::format_context`](crate::channel::SendContext::format_context).
pub struct FormatContext<'a> {
    _marker: PhantomData<&'a ()>,
}

impl FormatContext<'_> {
    pub(crate) fn new() -> Self {
        FormatContext {
            _marker: PhantomData,
        }
    }
}

impl<F: Format + ?Sized> Format for std::sync::Arc<F> {
    fn format(&self, input: &Input, context: &FormatContext) -> Result<String, FormatError> {
        self.as_ref().format(input, context)
    }
}

/// Builds a [`Format`] when its channel is built. Every `Format` is its own builder.
pub trait FormatBuilder: Debug + Send {
    fn build(self: Box<Self>, context: FormatBuildContext) -> Result<Box<dyn Format>, BuildErrors>;
}

/// What a format builder gets when its channel is built.
pub struct FormatBuildContext<'a> {
    channel_key: &'a str,
}

impl<'a> FormatBuildContext<'a> {
    pub(crate) fn new(channel_key: &'a str) -> Self {
        FormatBuildContext { channel_key }
    }

    /// The key of the format's channel, to name it in build errors.
    pub fn channel_key(&self) -> &'a str {
        self.channel_key
    }
}

impl FormatBuilder for Box<dyn FormatBuilder> {
    fn build(self: Box<Self>, context: FormatBuildContext) -> Result<Box<dyn Format>, BuildErrors> {
        (*self).build(context)
    }
}

impl<F: Format + 'static> FormatBuilder for F {
    fn build(
        self: Box<Self>,
        _context: FormatBuildContext,
    ) -> Result<Box<dyn Format>, BuildErrors> {
        Ok(self)
    }
}
