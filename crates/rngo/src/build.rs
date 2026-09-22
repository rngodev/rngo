pub use crate::channel::target::exec::ExecBuilder;
pub use crate::channel::target::stream::StreamBuilder;
use crate::channel::target::{Exec, Stream};
pub use crate::effect::schema::array::ArrayBuilder;
pub use crate::effect::schema::constant::ConstantBuilder;
pub use crate::effect::schema::context::ContextBuilder;
pub use crate::effect::schema::function::FunctionBuilder;
pub use crate::effect::schema::number::NumberBuilder;
pub use crate::effect::schema::object::ObjectBuilder;
pub use crate::effect::schema::reference::ReferenceBuilder;
pub use crate::effect::schema::select::SelectBuilder;
pub use crate::effect::schema::string::StrBuilder;
use crate::effect::schema::{
    Array, Constant, Context, Function, Number, Object, Reference, Select, Str,
};
pub use crate::format::SqlFormat;
use crate::signal::SqlSignal;
pub use crate::signal::sql::SqlSignalBuilder;
use thiserror::Error;

pub fn array() -> ArrayBuilder {
    Array::builder()
}

pub fn constant() -> ConstantBuilder {
    Constant::builder()
}

pub fn context() -> ContextBuilder {
    Context::builder()
}

pub fn function() -> FunctionBuilder {
    Function::builder()
}

pub fn number() -> NumberBuilder {
    Number::builder()
}

pub fn object() -> ObjectBuilder {
    Object::builder()
}

pub fn reference() -> ReferenceBuilder {
    Reference::builder()
}

pub fn select() -> SelectBuilder {
    Select::builder()
}

pub fn string() -> StrBuilder {
    Str::builder()
}

pub fn sql_signal() -> SqlSignalBuilder {
    SqlSignal::builder()
}

pub fn sql_format() -> SqlFormat {
    SqlFormat::builder()
}

pub fn stream() -> StreamBuilder {
    Stream::builder()
}

pub fn exec() -> ExecBuilder {
    Exec::builder()
}

#[derive(Error, Debug)]
#[error("failed to build: `{message}`")]
pub enum BuildError {
    MergeEffect {
        key: MergeEffectKey,
        message: String,
    },
    Effect {
        effect: String,
        key: EffectKey,
        message: String,
    },
    Schema {
        effect: String,
        path: Vec<SchemaEdge>,
        message: String,
    },
    Proxy {
        message: String,
    },
    Channel {
        channel: String,
        message: String,
    },
    ChannelTarget {
        channel: String,
        message: String,
    },
    Signal {
        signal: String,
        message: String,
    },
    Audit {
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct SchemaEdge {
    pub kind: &'static str,
    pub key: String,
}

#[derive(Debug)]
pub enum MergeEffectKey {
    Start,
    End,
}

#[derive(Debug)]
pub enum EffectKey {
    Schema,
    Trigger,
    Config,
    Start,
    End,
}
