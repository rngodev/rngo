use crate::proxy::format::Format;
use crate::spec::{self, ParseError};
use serde_json::Value;
use std::rc::Rc;

pub trait FormatParser {
    fn key(&self) -> &str;
    fn parse(
        &self,
        format: &spec::Format,
        spec: &spec::Spec,
    ) -> Result<Box<dyn Format>, Vec<ParseError>>;
}

/// Lets a channel parser build the optional `format` of its channel with the dialect's format parsers.
pub struct FormatParseContext<'a> {
    parsers: &'a [Box<dyn FormatParser>],
    spec: &'a spec::Spec,
}

impl<'a> FormatParseContext<'a> {
    pub fn new(parsers: &'a [Box<dyn FormatParser>], spec: &'a spec::Spec) -> Self {
        FormatParseContext { parsers, spec }
    }

    /// Parses `value` as a format; `None` if it has no `type`. Error paths are prefixed with
    /// `path`, where `value` sits relative to the caller's own node.
    pub fn parse(
        &self,
        path: &[&str],
        value: &Value,
    ) -> Result<Option<Rc<dyn Format>>, Vec<ParseError>> {
        let invalid = |message: String| vec![ParseError::invalid(message).at(path.iter().copied())];

        let format: spec::Format =
            serde_json::from_value(value.clone()).map_err(|e| invalid(e.to_string()))?;

        let Some(ftype) = format.ftype.as_deref() else {
            return Ok(None);
        };

        let matching: Vec<_> = self.parsers.iter().filter(|p| p.key() == ftype).collect();

        let result = match matching.as_slice() {
            [parser] => parser.parse(&format, self.spec),
            [] => {
                return Err(invalid("unknown format type".into()));
            }
            _ => {
                return Err(invalid(format!(
                    "{} format parsers matched",
                    matching.len()
                )));
            }
        };

        result
            .map(|format| Some(Rc::from(format)))
            .map_err(|errors| {
                errors
                    .into_iter()
                    .map(|error| error.prefixed(path))
                    .collect()
            })
    }
}
