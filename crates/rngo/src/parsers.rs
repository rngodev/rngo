//! The built-in parsers, for assembling a custom [`Dialect`](crate::Dialect) with [`Dialect::new`](crate::Dialect::new).

use crate::audit::signal::SqlSignal;
use crate::effect::generator::{
    Array, Constant, Context, Function, Number, Object, Reference, Select, Str,
};
use crate::parse::{ChannelParser, FormatParser, GeneratorParser, SignalParser};
use crate::proxy::channel::{Exec, Stdout, Stream};
use crate::proxy::format::{SqlFormat, TemplateFormat};

/// The `array` generator parser.
pub fn array() -> impl GeneratorParser {
    Array::parser()
}

/// The `constant` generator parser.
pub fn constant() -> impl GeneratorParser {
    Constant::parser()
}

/// The `context` generator parser.
pub fn context() -> impl GeneratorParser {
    Context::parser()
}

/// The `function` generator parser.
pub fn function() -> impl GeneratorParser {
    Function::parser()
}

/// The `number` generator parser.
pub fn number() -> impl GeneratorParser {
    Number::parser()
}

/// The `object` generator parser.
pub fn object() -> impl GeneratorParser {
    Object::parser()
}

/// The `reference` generator parser.
pub fn reference() -> impl GeneratorParser {
    Reference::parser()
}

/// The `select` generator parser.
pub fn select() -> impl GeneratorParser {
    Select::parser()
}

/// The `string` generator parser.
pub fn string() -> impl GeneratorParser {
    Str::parser()
}

/// The `sql` format parser.
pub fn sql_format() -> impl FormatParser {
    SqlFormat::parser()
}

/// The `template` format parser.
pub fn template_format() -> impl FormatParser {
    TemplateFormat::parser()
}

/// The `exec` channel parser.
pub fn exec() -> impl ChannelParser {
    Exec::parser()
}

/// The `stdout` channel parser.
pub fn stdout() -> impl ChannelParser {
    Stdout::parser()
}

/// The `stream` channel parser.
pub fn stream() -> impl ChannelParser {
    Stream::parser()
}

/// The `sql` signal parser.
pub fn sql_signal() -> impl SignalParser {
    SqlSignal::parser()
}
