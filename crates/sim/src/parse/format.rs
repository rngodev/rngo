use crate::format::Format;
use crate::spec::{self, ParseError};

pub trait FormatParser {
    fn should_parse(&self, format: &spec::Format) -> bool;
    fn parse(&self, format: &spec::Format) -> Result<Box<dyn Format>, Vec<ParseError>>;
}
