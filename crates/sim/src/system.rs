use crate::format::Format;
use crate::spec::SystemImport;

#[derive(Debug)]
pub struct System {
    pub key: String,
    pub format: Option<Box<dyn Format>>,
    pub import: SystemImport,
}
