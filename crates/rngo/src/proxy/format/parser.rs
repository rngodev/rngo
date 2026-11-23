use crate::error::ParseErrors;
use crate::proxy::format::FormatBuilder;
use crate::spec;

/// Parses spec formats whose `type` is its `key` into format builders.
pub trait FormatParser: Send + Sync {
    fn key(&self) -> &str;
    fn parse(
        &self,
        format: &spec::Format,
        context: &FormatParseContext,
    ) -> Result<Box<dyn FormatBuilder>, ParseErrors>;
}

/// What a format parser sees while parsing a format.
pub struct FormatParseContext<'a> {
    spec: &'a spec::Spec,
}

impl<'a> FormatParseContext<'a> {
    pub(crate) fn new(spec: &'a spec::Spec) -> Self {
        FormatParseContext { spec }
    }

    /// The whole spec being parsed.
    pub fn spec(&self) -> &'a spec::Spec {
        self.spec
    }
}
