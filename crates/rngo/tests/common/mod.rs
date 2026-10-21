#![allow(dead_code)]

use rngo::{BuildError, GeneratorEdge, ParseError};

pub trait BuildErrorTestExt {
    fn message(&self) -> &str;
    fn generator_path(&self) -> Option<&Vec<GeneratorEdge>>;
}

impl BuildErrorTestExt for BuildError {
    fn message(&self) -> &str {
        match self {
            BuildError::Simulation { message, .. } => message,
            BuildError::Effect { message, .. } => message,
            BuildError::Generator { message, .. } => message,
            BuildError::Proxy { message } => message,
            BuildError::Channel { message, .. } => message,
            BuildError::Signal { message, .. } => message,
            BuildError::Audit { message } => message,
        }
    }

    fn generator_path(&self) -> Option<&Vec<GeneratorEdge>> {
        match self {
            BuildError::Generator { path, .. } => Some(path),
            _ => None,
        }
    }
}

pub trait ParseErrorTestExt {
    fn message(&self) -> &str;
    fn path(&self) -> Option<&Vec<String>>;
}

impl ParseErrorTestExt for ParseError {
    fn message(&self) -> &str {
        let ParseError::Invalid { message, .. } = self;
        message
    }

    fn path(&self) -> Option<&Vec<String>> {
        let ParseError::Invalid { path, .. } = self;
        path.as_ref()
    }
}
