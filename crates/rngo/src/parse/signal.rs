use crate::audit::signal::Signal;
use crate::error::ParseError;
use crate::spec;

pub trait SignalParser {
    fn key(&self) -> &str;
    fn parse(&self, key: &str, signal: &spec::Signal) -> Result<Box<dyn Signal>, Vec<ParseError>>;
}
