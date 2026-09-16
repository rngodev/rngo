use crate::signal::Signal;
use crate::spec::{self, ParseError};

pub trait SignalParser {
    fn key(&self) -> &str;
    fn parse(&self, key: &str, signal: &spec::Signal) -> Result<Box<dyn Signal>, Vec<ParseError>>;
}
