use super::{
    Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext, GeneratorResult,
};
use crate::build::{BuildError, GeneratorEdge};
use crate::cel::{CelContextExt, json_to_cel};
use crate::effect::generator::Metadata;
use crate::parse::{GeneratorParseVisitor, GeneratorParser};
use crate::spec::{self, ParseError as Error};
use cel::{Context, Program};
use indexmap::IndexMap;
use std::collections::HashMap;

#[derive(Debug)]
pub struct Function {
    program: Program,
    variables: HashMap<String, Box<dyn Generator>>,
}

impl Function {
    pub fn builder() -> FunctionBuilder {
        FunctionBuilder {
            expression: None,
            variable_builders: IndexMap::new(),
        }
    }

    pub fn parser() -> FunctionParser {
        FunctionParser {}
    }
}

impl Generator for Function {
    fn next(&mut self, context: &GeneratorContext) -> GeneratorResult {
        let mut ctx = Context::default();
        ctx.with_strings();

        let mut complete = true;
        let mut metadata: Vec<Metadata> = Vec::new();

        for (key, generator) in &mut self.variables {
            let result = generator.next(context);

            if let Some(value) = result.value {
                ctx.add_variable(key.as_str(), json_to_cel(value));
            } else {
                complete = false;
            }

            for mut result_metadata in result.metadata {
                result_metadata.prefix_attribute(key.clone().into());
                metadata.push(result_metadata)
            }
        }

        if !complete {
            return GeneratorResult {
                value: None,
                metadata,
            };
        }

        match self.program.execute(&ctx) {
            Ok(result) => GeneratorResult {
                value: Some(cel_to_json(result)),
                metadata,
            },
            Err(e) => {
                metadata.push(Metadata {
                    mtype: "error".into(),
                    attribute: None,
                    data: Some(serde_json::json!({ "message": e.to_string() })),
                });
                GeneratorResult {
                    value: None,
                    metadata,
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct FunctionBuilder {
    expression: Option<String>,
    variable_builders: IndexMap<String, Box<dyn GeneratorBuilder>>,
}

impl FunctionBuilder {
    pub fn expression(mut self, expression: impl Into<String>) -> Self {
        self.set_expression(expression);
        self
    }

    pub fn set_expression(&mut self, expression: impl Into<String>) -> &mut Self {
        self.expression = Some(expression.into());
        self
    }

    pub fn variable(
        mut self,
        key: impl Into<String>,
        builder: impl GeneratorBuilder + 'static,
    ) -> Self {
        self.set_variable(key, builder);
        self
    }

    pub fn set_variable(
        &mut self,
        key: impl Into<String>,
        builder: impl GeneratorBuilder + 'static,
    ) -> &mut Self {
        self.variable_builders.insert(key.into(), Box::new(builder));
        self
    }
}

impl GeneratorBuilder for FunctionBuilder {
    fn build(&self, visitor: GeneratorBuildVisitor) -> Result<Box<dyn Generator>, Vec<BuildError>> {
        let expression = match &self.expression {
            Some(e) => e,
            None => return Err(vec![visitor.error("expression was not set")]),
        };

        let mut errors = vec![];
        let mut variables = HashMap::new();

        for (key, builder) in &self.variable_builders {
            let var_visitor = visitor.follow_edge(GeneratorEdge {
                kind: "variable",
                key: key.clone(),
            });
            match builder.build(var_visitor) {
                Ok(generator) => {
                    variables.insert(key.clone(), generator);
                }
                Err(mut e) => errors.append(&mut e),
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        let program = Program::compile(expression)
            .map_err(|e| vec![visitor.error(format!("expression compilation failed: {e}"))])?;

        Ok(Box::new(Function { program, variables }))
    }
}

pub struct FunctionParser {}

impl GeneratorParser for FunctionParser {
    fn key(&self) -> &str {
        "function"
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<Error>> {
        let expression = match visitor.spec().fields.get("expression") {
            Some(v) if v.is_string() => v.as_str().unwrap().to_string(),
            Some(_) => {
                return Err(vec![
                    visitor.input_error("expression", "expression must be a string"),
                ]);
            }
            None => {
                return Err(vec![visitor.invalid("expression must be specified")]);
            }
        };

        let mut builder = Function::builder();
        builder.set_expression(expression);

        if let Some(vars_value) = visitor.spec().fields.get("variables") {
            let vars: IndexMap<String, spec::Generator> =
                serde_json::from_value(vars_value.clone()).map_err(|e| {
                    vec![visitor.input_error("variables", format!("variables parsing failed: {e}"))]
                })?;

            let mut errors = vec![];
            for (key, generator) in vars {
                let path = vec!["variables".into(), key.clone()];
                match visitor.parse_input_generator(path, generator) {
                    Ok(b) => {
                        builder.set_variable(key, b);
                    }
                    Err(mut e) => errors.append(&mut e),
                }
            }

            if !errors.is_empty() {
                return Err(errors);
            }
        }

        Ok(Box::new(builder))
    }
}

fn cel_to_json(cel_value: cel::Value) -> serde_json::Value {
    match cel_value {
        cel::Value::Bool(b) => b.into(),
        cel::Value::Int(i) => i.into(),
        cel::Value::UInt(u) => u.into(),
        cel::Value::Float(d) => d.into(),
        cel::Value::String(s) => serde_json::value::Value::String((*s).clone()),
        cel::Value::Bytes(b) => {
            let hex_string: String = b.iter().map(|b| format!("{b:02x}")).collect();
            format!("[hex:{hex_string}]").into()
        }
        cel::Value::Null => serde_json::Value::Null,
        cel::Value::List(l) => (*l).clone().into_iter().map(cel_to_json).collect(),
        cel::Value::Map(m) => {
            let map = serde_json::Map::from_iter(
                (*m.map)
                    .clone()
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), cel_to_json(v))),
            );
            serde_json::value::Value::Object(map)
        }
        cel::Value::Duration(d) => format!("[duration:{}]", d.num_seconds()).into(),
        cel::Value::Timestamp(t) => t.to_rfc3339().into(),
        cel::Value::Function(name, _) => format!("[function:{name}]").into(),
        cel::Value::Opaque(opaque) => format!("[opaque:{}]", opaque.runtime_type_name()).into(),
    }
}
