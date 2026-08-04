use crate::format::Format;
use crate::spec::{self, ParseError};

pub trait FormatParser {
    fn key(&self) -> &str;
    fn parse(&self, format: &spec::Format) -> Result<Box<dyn Format>, Vec<ParseError>>;
}
