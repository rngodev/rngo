use crate::signal::Signal;
use rngo_core::spec::{self, ParseError};

pub trait SignalParser {
    fn key(&self) -> &str;
    fn parse(&self, key: &str, signal: &spec::Signal) -> Result<Box<dyn Signal>, Vec<ParseError>>;
}
