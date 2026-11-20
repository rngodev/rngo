use crate::effect::generator::GeneratorEdge;
use std::fmt;
use std::ops::Deref;
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
#[non_exhaustive]
pub enum ParseErrorKind {
    /// A free-form problem, described by its message.
    #[error("{0}")]
    Invalid(String),
    /// No parser matched the `type`; `kind` is what was being parsed (e.g. `generator`).
    #[error("unknown {kind} type")]
    UnknownType { kind: &'static str },
    /// More than one parser matched the `type`; `count` is how many did.
    #[error("{count} {kind} parsers matched")]
    AmbiguousType { kind: &'static str, count: usize },
    /// The spec names a channel it doesn't define.
    #[error("unknown channel: {0}")]
    UnknownChannel(String),
    /// The spec's `version` is `found`, but this rngo reads only `supported`.
    #[error(
        "spec version {found} is not supported; this rngo reads version {supported} (see {})",
        crate::spec::MIGRATION_URL
    )]
    UnsupportedVersion { found: u64, supported: u32 },
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

    /// What went wrong.
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
#[non_exhaustive]
pub enum BuildError {
    #[error("failed to build simulation: `{message}`")]
    Simulation { key: SimulationKey, message: String },
    #[error("failed to build `effects.{effect}{}`: `{message}`", key.path_suffix())]
    Effect {
        effect: String,
        key: EffectKey,
        message: String,
    },
    #[error("failed to build `effects.{effect}.generator`: `{message}`")]
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
    /// An error in the simulation's own settings; `key` says which.
    pub fn simulation(key: SimulationKey, message: impl Into<String>) -> Self {
        BuildError::Simulation {
            key,
            message: message.into(),
        }
    }

    /// An error in effect `effect`; `key` is the part of its spec it concerns.
    pub fn effect(effect: impl Into<String>, key: EffectKey, message: impl Into<String>) -> Self {
        BuildError::Effect {
            effect: effect.into(),
            key,
            message: message.into(),
        }
    }

    /// An error in a generator of effect `effect`; `path` leads from the effect's generator to the failing one.
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

    /// An error in the proxy as a whole.
    pub fn proxy(message: impl Into<String>) -> Self {
        BuildError::Proxy {
            message: message.into(),
        }
    }

    /// An error in channel `channel`.
    pub fn channel(channel: impl Into<String>, message: impl Into<String>) -> Self {
        BuildError::Channel {
            channel: channel.into(),
            message: message.into(),
        }
    }

    /// An error in signal `signal`.
    pub fn signal(signal: impl Into<String>, message: impl Into<String>) -> Self {
        BuildError::Signal {
            signal: signal.into(),
            message: message.into(),
        }
    }

    /// An error in the audit as a whole.
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

macro_rules! error_list {
    ($(#[$meta:meta])* $name:ident, $item:ty) => {
        $(#[$meta])*
        #[derive(Debug, Default)]
        pub struct $name(Vec<$item>);

        impl $name {
            /// The individual errors.
            pub fn as_slice(&self) -> &[$item] {
                &self.0
            }

            /// The individual errors, as an owned `Vec`.
            pub fn into_vec(self) -> Vec<$item> {
                self.0
            }

            /// Appends one error.
            pub fn push(&mut self, error: $item) {
                self.0.push(error);
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                for (i, error) in self.0.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{error}")?;
                }
                Ok(())
            }
        }

        impl std::error::Error for $name {}

        impl Deref for $name {
            type Target = [$item];

            fn deref(&self) -> &[$item] {
                &self.0
            }
        }

        impl From<$item> for $name {
            fn from(error: $item) -> Self {
                $name(vec![error])
            }
        }

        impl From<Vec<$item>> for $name {
            fn from(errors: Vec<$item>) -> Self {
                $name(errors)
            }
        }

        impl From<$name> for Vec<$item> {
            fn from(errors: $name) -> Self {
                errors.0
            }
        }

        impl FromIterator<$item> for $name {
            fn from_iter<I: IntoIterator<Item = $item>>(iter: I) -> Self {
                $name(iter.into_iter().collect())
            }
        }

        impl Extend<$item> for $name {
            fn extend<I: IntoIterator<Item = $item>>(&mut self, iter: I) {
                self.0.extend(iter);
            }
        }

        impl IntoIterator for $name {
            type Item = $item;
            type IntoIter = std::vec::IntoIter<$item>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.into_iter()
            }
        }

        impl<'a> IntoIterator for &'a $name {
            type Item = &'a $item;
            type IntoIter = std::slice::Iter<'a, $item>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.iter()
            }
        }
    };
}

error_list!(
    /// Every problem found while parsing a spec, displayed one per line.
    ParseErrors,
    ParseError
);

error_list!(
    /// Every problem found while building a simulation, proxy or audit, displayed one per line.
    BuildErrors,
    BuildError
);

impl ParseErrors {
    /// Prepends `prefix` to every error's path; see [`ParseError::prefixed`].
    pub fn prefixed(self, prefix: &[&str]) -> Self {
        self.into_iter().map(|e| e.prefixed(prefix)).collect()
    }
}

/// The part of a simulation's settings an error concerns.
#[derive(Debug)]
#[non_exhaustive]
pub enum SimulationKey {
    /// The default prototype's `start`.
    Start,
    /// The default prototype's `end`.
    End,
    /// The run log the simulation reads earlier inputs from.
    RunLogReader,
}

/// The part of an effect's spec an error concerns.
#[derive(Debug)]
#[non_exhaustive]
pub enum EffectKey {
    /// The effect's `generator`.
    Generator,
    /// The effect's `trigger`.
    Trigger,
    /// The effect as a whole.
    Config,
    /// The effect's `start`.
    Start,
    /// The effect's `end`.
    End,
}

impl EffectKey {
    /// The field's name, appended to the effect's path; empty for `Config`, which concerns the effect as a whole.
    fn path_suffix(&self) -> &'static str {
        match self {
            EffectKey::Generator => ".generator",
            EffectKey::Trigger => ".trigger",
            EffectKey::Config => "",
            EffectKey::Start => ".start",
            EffectKey::End => ".end",
        }
    }
}

/// A spec problem a simulation finds partway through, which stops it; inputs logged before it are kept.
#[derive(Error, Debug)]
#[error("simulation failed at `effects.{effect}{}`: `{message}`", key.path_suffix())]
pub struct SimulationError {
    effect: String,
    key: EffectKey,
    message: String,
}

impl SimulationError {
    pub(crate) fn new(
        effect: impl Into<String>,
        key: EffectKey,
        message: impl Into<String>,
    ) -> Self {
        SimulationError {
            effect: effect.into(),
            key,
            message: message.into(),
        }
    }

    /// The key of the effect that failed.
    pub fn effect(&self) -> &str {
        &self.effect
    }

    /// The part of the effect's spec the error concerns.
    pub fn key(&self) -> &EffectKey {
        &self.key
    }

    /// The error's message, without the effect it concerns.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A failure reading from or writing to a run log.
#[derive(Error, Debug)]
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
pub enum ChannelError {
    #[error("channel `{channel}`: failed to write: {source}")]
    Write {
        channel: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "channel `{channel}`: command exited ({status}){}",
        format_stderr(stderr)
    )]
    Exited {
        channel: String,
        status: String,
        stderr: String,
    },
    #[error("channel `{channel}`: failed to run command: {source}")]
    Spawn {
        channel: String,
        #[source]
        source: std::io::Error,
    },
}

fn format_stderr(stderr: &str) -> String {
    if stderr.is_empty() {
        String::new()
    } else {
        format!(":\n{stderr}")
    }
}

/// A failure sending an input through a proxy.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ProxyError {
    #[error(transparent)]
    Channel(#[from] ChannelError),
    #[error(transparent)]
    RunLog(#[from] RunLogError),
}

/// A failure evaluating a signal.
#[derive(Error, Debug)]
#[non_exhaustive]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn two_errors() -> ParseErrors {
        [
            ParseError::invalid("first").at(["a"]),
            ParseError::invalid("second").at(["b", "c"]),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn displays_one_error_per_line() {
        assert_eq!(
            two_errors().to_string(),
            "failed to parse `a`: `first`\nfailed to parse `b.c`: `second`"
        );
    }

    #[test]
    fn propagates_with_question_mark_into_box_dyn_error() {
        fn parse() -> Result<(), Box<dyn std::error::Error>> {
            Err(two_errors())?
        }

        assert_eq!(parse().unwrap_err().to_string(), two_errors().to_string());
    }

    #[test]
    fn exposes_individual_errors() {
        let errors = two_errors();
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[1].path(), ["b", "c"]);
        let messages: Vec<_> = errors.into_iter().map(|e| e.message()).collect();
        assert_eq!(messages, ["first", "second"]);
    }

    #[test]
    fn prefixes_every_error() {
        let paths: Vec<_> = two_errors()
            .prefixed(&["channels", "x"])
            .iter()
            .map(|e| e.path().join("."))
            .collect();
        assert_eq!(paths, ["channels.x.a", "channels.x.b.c"]);
    }

    #[test]
    fn converts_a_single_build_error() {
        let errors: BuildErrors = BuildError::proxy("broken").into();
        assert_eq!(errors.to_string(), "failed to build proxy: `broken`");
    }
}
