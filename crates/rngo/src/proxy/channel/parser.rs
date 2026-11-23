use crate::error::{ParseError, ParseErrorKind, ParseErrors};
use crate::proxy::channel::ChannelBuilder;
use crate::proxy::format::{FormatBuilder, FormatParseContext, FormatParser};
use crate::spec;
use serde_json::Value;

pub trait ChannelParser: Send + Sync {
    fn key(&self) -> &str;

    /// Parses `channel` into a channel builder; `context` parses its `format` field for channels
    /// that accept one. Error paths are relative to the channel.
    fn parse(
        &self,
        channel: &spec::Channel,
        context: &ChannelParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, ParseErrors>;
}

/// What a channel parser sees while parsing a channel, including the dialect's format parsers for
/// its optional `format`.
pub struct ChannelParseContext<'a> {
    parsers: &'a [Box<dyn FormatParser>],
    spec: &'a spec::Spec,
}

impl<'a> ChannelParseContext<'a> {
    pub(crate) fn new(parsers: &'a [Box<dyn FormatParser>], spec: &'a spec::Spec) -> Self {
        ChannelParseContext { parsers, spec }
    }

    /// The whole spec being parsed.
    pub fn spec(&self) -> &'a spec::Spec {
        self.spec
    }

    /// Parses `value` as a format; `None` if it has no `type`. Error paths are prefixed with
    /// `path`, where `value` sits relative to the caller's own node.
    pub fn parse_format(
        &self,
        path: &[&str],
        value: &Value,
    ) -> Result<Option<Box<dyn FormatBuilder>>, ParseErrors> {
        let invalid = |message: String| {
            ParseErrors::from(ParseError::invalid(message).at(path.iter().copied()))
        };

        let format: spec::Format =
            serde_json::from_value(value.clone()).map_err(|e| invalid(e.to_string()))?;

        let Some(kind) = format.kind.as_deref() else {
            return Ok(None);
        };

        let matching: Vec<_> = self.parsers.iter().filter(|p| p.key() == kind).collect();

        let type_error = |kind: ParseErrorKind| {
            ParseErrors::from(ParseError::new(kind).at(path.iter().copied().chain(["type"])))
        };

        let result = match matching.as_slice() {
            [parser] => parser.parse(&format, &FormatParseContext::new(self.spec)),
            [] => {
                return Err(type_error(ParseErrorKind::UnknownType {
                    kind: "format",
                    found: Some(kind.to_string()),
                }));
            }
            _ => {
                return Err(type_error(ParseErrorKind::AmbiguousType {
                    kind: "format",
                    count: matching.len(),
                }));
            }
        };

        result.map(Some).map_err(|errors| {
            errors
                .into_iter()
                .map(|error| error.prefixed(path))
                .collect()
        })
    }
}
