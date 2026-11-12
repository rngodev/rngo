pub(crate) use crate::error::{BuildError, EffectKey, SimulationKey};

#[cfg(test)]
pub(crate) use test_dsl::*;

/// Builder-style constructors for specs in tests; the public API takes specs through `Dialect`.
#[cfg(test)]
mod test_dsl {
    use crate::audit::signal::SqlSignal;
    pub(crate) use crate::audit::signal::sql::SqlSignalBuilder;
    pub(crate) use crate::effect::generator::array::ArrayBuilder;
    pub(crate) use crate::effect::generator::constant::ConstantBuilder;
    pub(crate) use crate::effect::generator::context::ContextBuilder;
    pub(crate) use crate::effect::generator::function::FunctionBuilder;
    pub(crate) use crate::effect::generator::number::NumberBuilder;
    pub(crate) use crate::effect::generator::object::ObjectBuilder;
    pub(crate) use crate::effect::generator::reference::ReferenceBuilder;
    pub(crate) use crate::effect::generator::select::SelectBuilder;
    pub(crate) use crate::effect::generator::string::StrBuilder;
    use crate::effect::generator::{
        Array, Constant, Context, Function, Number, Object, Reference, Select, Str,
    };

    pub(crate) fn array() -> ArrayBuilder {
        Array::builder()
    }

    pub(crate) fn constant() -> ConstantBuilder {
        Constant::builder()
    }

    pub(crate) fn context() -> ContextBuilder {
        Context::builder()
    }

    pub(crate) fn function() -> FunctionBuilder {
        Function::builder()
    }

    pub(crate) fn number() -> NumberBuilder {
        Number::builder()
    }

    pub(crate) fn object() -> ObjectBuilder {
        Object::builder()
    }

    pub(crate) fn reference() -> ReferenceBuilder {
        Reference::builder()
    }

    pub(crate) fn select() -> SelectBuilder {
        Select::builder()
    }

    pub(crate) fn string() -> StrBuilder {
        Str::builder()
    }

    pub(crate) fn sql_signal() -> SqlSignalBuilder {
        SqlSignal::builder()
    }
}
