use crate::audit::signal::SqlSignal;
pub use crate::audit::signal::sql::SqlSignalBuilder;
pub use crate::effect::generator::array::ArrayBuilder;
pub use crate::effect::generator::constant::ConstantBuilder;
pub use crate::effect::generator::context::ContextBuilder;
pub use crate::effect::generator::function::FunctionBuilder;
pub use crate::effect::generator::number::NumberBuilder;
pub use crate::effect::generator::object::ObjectBuilder;
pub use crate::effect::generator::reference::ReferenceBuilder;
pub use crate::effect::generator::select::SelectBuilder;
pub use crate::effect::generator::string::StrBuilder;
use crate::effect::generator::{
    Array, Constant, Context, Function, Number, Object, Reference, Select, Str,
};
pub use crate::proxy::channel::exec::ExecBuilder;
pub use crate::proxy::channel::stdout::StdoutBuilder;
pub use crate::proxy::channel::stream::StreamBuilder;
use crate::proxy::channel::{Exec, Stdout, Stream};
pub use crate::proxy::format::{SqlFormat, TemplateFormat};

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

pub fn stdout() -> StdoutBuilder {
    Stdout::builder()
}

pub use crate::error::{BuildError, EffectKey, GeneratorEdge, SimulationKey};
