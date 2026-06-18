#![allow(dead_code)]

use rngo_sim::{BuildError, SchemaEdge, SpecError};

pub trait BuildErrorTestExt {
    fn message(&self) -> &str;
    fn schema_path(&self) -> Option<&Vec<SchemaEdge>>;
}

impl BuildErrorTestExt for BuildError {
    fn message(&self) -> &str {
        match self {
            BuildError::Simulation { message, .. } => message,
            BuildError::Effect { message, .. } => message,
            BuildError::Schema { message, .. } => message,
        }
    }

    fn schema_path(&self) -> Option<&Vec<SchemaEdge>> {
        match self {
            BuildError::Schema { path, .. } => Some(path),
            _ => None,
        }
    }
}

pub trait SpecErrorTestExt {
    fn message(&self) -> &str;
    fn path(&self) -> Option<&Vec<String>>;
}

impl SpecErrorTestExt for SpecError {
    fn message(&self) -> &str {
        let SpecError { message, .. } = self;
        message
    }

    fn path(&self) -> Option<&Vec<String>> {
        let SpecError { path, .. } = self;
        path.as_ref()
    }
}
