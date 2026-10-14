use crate::audit::probe::Probe;
use crate::spec::{self, ParseError};

pub trait ProbeParser {
    fn key(&self) -> &str;
    fn parse(&self, key: &str, probe: &spec::Probe) -> Result<Box<dyn Probe>, Vec<ParseError>>;
}
