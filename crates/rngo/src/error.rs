use crate::effect::generator::GeneratorEdge;
use thiserror::Error;

/// A problem found while parsing a spec, located by the path of the field it concerns.
#[derive(Error, Debug)]
#[error("failed to parse{}: `{kind}`", display_path(.path))]
pub struct ParseError {
    path: Vec<String>,
    kind: ParseErrorKind,
}

/// What went wrong while parsing a spec.
#[derive(Error, Debug)]
pub enum ParseErrorKind {
    #[error("{0}")]
    Invalid(String),
    #[error("unknown {kind} type")]
    UnknownType { kind: &'static str },
    #[error("{count} {kind} parsers matched")]
    AmbiguousType { kind: &'static str, count: usize },
    #[error("unknown channel: {0}")]
    UnknownChannel(String),
}

impl ParseError {
    /// An error of `kind` with no path, i.e. one that concerns the document being parsed itself.
    pub fn new(kind: ParseErrorKind) -> Self {
        ParseError { path: vec![], kind }
    }

    /// An error with the free-form `message` and no path.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ParseErrorKind::Invalid(message.into()))
    }

    /// Replaces the error's path.
    pub fn at<S: Into<String>>(mut self, path: impl IntoIterator<Item = S>) -> Self {
        self.path = path.into_iter().map(Into::into).collect();
        self
    }

    /// Prepends `prefix` to the error's path, so an error from a nested parser says where it
    /// came from.
    pub fn prefixed(mut self, prefix: &[&str]) -> Self {
        self.path.splice(0..0, prefix.iter().map(|s| s.to_string()));
        self
    }

    /// The path of the field the error concerns; empty if it concerns the whole document.
    pub fn path(&self) -> &[String] {
        &self.path
    }

    pub fn kind(&self) -> &ParseErrorKind {
        &self.kind
    }

    /// The error's message, without its path.
    pub fn message(&self) -> String {
        self.kind.to_string()
    }
}

fn display_path(path: &[String]) -> String {
    if path.is_empty() {
        String::new()
    } else {
        format!(" `{}`", path.join("."))
    }
}

/// A problem found while turning parsed builders into runnable simulations, proxies or audits.
#[derive(Error, Debug)]
pub enum BuildError {
    #[error("failed to build simulation: `{message}`")]
    Simulation { key: SimulationKey, message: String },
    #[error("failed to build effect `{effect}`: `{message}`")]
    Effect {
        effect: String,
        key: EffectKey,
        message: String,
    },
    #[error("failed to build generator of effect `{effect}`: `{message}`")]
    Generator {
        effect: String,
        path: Vec<GeneratorEdge>,
        message: String,
    },
    #[error("failed to build proxy: `{message}`")]
    Proxy { message: String },
    #[error("failed to build channel `{channel}`: `{message}`")]
    Channel { channel: String, message: String },
    #[error("failed to build signal `{signal}`: `{message}`")]
    Signal { signal: String, message: String },
    #[error("failed to build audit: `{message}`")]
    Audit { message: String },
}

impl BuildError {
    pub fn simulation(key: SimulationKey, message: impl Into<String>) -> Self {
        BuildError::Simulation {
            key,
            message: message.into(),
        }
    }

    pub fn effect(effect: impl Into<String>, key: EffectKey, message: impl Into<String>) -> Self {
        BuildError::Effect {
            effect: effect.into(),
            key,
            message: message.into(),
        }
    }

    pub fn generator(
        effect: impl Into<String>,
        path: Vec<GeneratorEdge>,
        message: impl Into<String>,
    ) -> Self {
        BuildError::Generator {
            effect: effect.into(),
            path,
            message: message.into(),
        }
    }

    pub fn proxy(message: impl Into<String>) -> Self {
        BuildError::Proxy {
            message: message.into(),
        }
    }

    pub fn channel(channel: impl Into<String>, message: impl Into<String>) -> Self {
        BuildError::Channel {
            channel: channel.into(),
            message: message.into(),
        }
    }

    pub fn signal(signal: impl Into<String>, message: impl Into<String>) -> Self {
        BuildError::Signal {
            signal: signal.into(),
            message: message.into(),
        }
    }

    pub fn audit(message: impl Into<String>) -> Self {
        BuildError::Audit {
            message: message.into(),
        }
    }

    /// The error's message, without the part of the simulation it concerns.
    pub fn message(&self) -> &str {
        match self {
            BuildError::Simulation { message, .. }
            | BuildError::Effect { message, .. }
            | BuildError::Generator { message, .. }
            | BuildError::Proxy { message }
            | BuildError::Channel { message, .. }
            | BuildError::Signal { message, .. }
            | BuildError::Audit { message } => message,
        }
    }

    /// The path through the generator tree to the generator that failed, for generator errors.
    pub fn generator_path(&self) -> Option<&[GeneratorEdge]> {
        match self {
            BuildError::Generator { path, .. } => Some(path),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum SimulationKey {
    Start,
    End,
    RunLogReader,
}

#[derive(Debug)]
pub enum EffectKey {
    Generator,
    Trigger,
    Config,
    Start,
    End,
}

/// A failure reading from or writing to a run log.
#[derive(Error, Debug)]
pub enum RunLogError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("this run log does not support running queries")]
    Unsupported,
    #[error("the query's result can't be represented as JSON")]
    UnrepresentableValue,
}

/// A failure turning an input into the string a channel delivers.
#[derive(Error, Debug)]
pub enum FormatError {
    #[error("failed to serialize the input: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("invalid template: {0}")]
    Template(String),
    #[error("{0}")]
    Render(String),
}

/// A failure delivering an input to a channel, which the run can't carry on past.
#[derive(Error, Debug)]
pub enum ChannelError {
    #[error("channel `{channel}`: failed to write: {source}")]
    Write {
        channel: String,
        #[source]
        source: std::io::Error,
    },
    #[error("channel `{channel}`: failed to run command: {source}")]
    Spawn {
        channel: String,
        #[source]
        source: std::io::Error,
    },
}

/// A failure sending an input through a proxy.
#[derive(Error, Debug)]
pub enum ProxyError {
    #[error(transparent)]
    Channel(#[from] ChannelError),
    #[error(transparent)]
    RunLog(#[from] RunLogError),
}

/// A failure evaluating a signal.
#[derive(Error, Debug)]
pub enum SignalError {
    #[error("expect expression failed to compile: {0}")]
    Compile(String),
    #[error("expect expression failed to evaluate: {0}")]
    Evaluate(String),
    #[error("expect expression must evaluate to a bool, got {0}")]
    NotBool(String),
    #[error("query failed: {0}")]
    Query(#[from] RunLogError),
}
