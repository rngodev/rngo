use crate::proxy::format::Format;
use crate::spec::{self, ParseError};
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
pub struct FormatContext<'a> {
    pub(super) parsers: &'a [Box<dyn FormatParser>],
    pub(super) spec: &'a spec::Spec,
}

impl FormatContext<'_> {
    /// Parses `channel`'s `format` field, if it has one with a `type`. Error paths are relative to
    /// the channel.
    pub fn parse(
        &self,
        channel: &spec::Channel,
    ) -> Result<Option<Rc<dyn Format>>, Vec<ParseError>> {
        let Some(value) = channel.fields.get("format") else {
            return Ok(None);
        };

        let invalid = |message: String| {
            vec![ParseError::Invalid {
                path: Some(vec!["format".into()]),
                message,
            }]
        };

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
                    .map(|ParseError::Invalid { path, message }| {
                        let mut full = vec!["format".to_string()];
                        full.extend(path.unwrap_or_default());
                        ParseError::Invalid {
                            path: Some(full),
                            message,
                        }
                    })
                    .collect()
            })
    }
}
