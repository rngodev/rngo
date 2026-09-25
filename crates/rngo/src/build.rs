use crate::audit::signal::SqlSignal;
pub use crate::audit::signal::sql::SqlSignalBuilder;
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
pub use crate::proxy::channel::target::exec::ExecBuilder;
pub use crate::proxy::channel::target::stream::StreamBuilder;
use crate::proxy::channel::target::{Exec, Stream};
pub use crate::proxy::format::SqlFormat;
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
    Simulation {
        key: SimulationKey,
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
pub enum SimulationKey {
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
