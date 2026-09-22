#![allow(dead_code)]

use rngo::{BuildError, ParseError, SchemaEdge};

pub trait BuildErrorTestExt {
    fn message(&self) -> &str;
    fn schema_path(&self) -> Option<&Vec<SchemaEdge>>;
}

impl BuildErrorTestExt for BuildError {
    fn message(&self) -> &str {
        match self {
            BuildError::MergeEffect { message, .. } => message,
            BuildError::Effect { message, .. } => message,
            BuildError::Schema { message, .. } => message,
            BuildError::Proxy { message } => message,
            BuildError::Channel { message, .. } => message,
            BuildError::ChannelTarget { message, .. } => message,
            BuildError::Signal { message, .. } => message,
            BuildError::Audit { message } => message,
        }
    }

    fn schema_path(&self) -> Option<&Vec<SchemaEdge>> {
        match self {
            BuildError::Schema { path, .. } => Some(path),
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
        let ParseError::SchemaError { message, .. } = self;
        message
    }

    fn path(&self) -> Option<&Vec<String>> {
        let ParseError::SchemaError { path, .. } = self;
        path.as_ref()
    }
}
