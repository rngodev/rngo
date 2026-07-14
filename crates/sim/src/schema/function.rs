use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult};
use crate::build::{BuildError, SchemaEdge};
use crate::spec::{self, SchemaParseVisitor, SchemaParser, SpecError as Error};
use crate::util::cel::CelContextExt;
use cel::{Context, Program};
use indexmap::IndexMap;
use std::collections::HashMap;

#[derive(Debug)]
pub struct Function {
    program: Program,
    variables: HashMap<String, Box<dyn Schema>>,
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

impl Schema for Function {
    fn next(&mut self, context: &SchemaContext) -> SchemaResult {
        let mut ctx = Context::default();
        ctx.with_strings();
        for (key, schema) in &mut self.variables {
            match schema.next(context) {
                SchemaResult::Ok { value } => {
                    ctx.add_variable(key.as_str(), json_to_cel(value));
                }
                err => return err,
            }
        }
        match self.program.execute(&ctx) {
            Ok(result) => SchemaResult::Ok {
                value: cel_to_json(result),
            },
            Err(e) => SchemaResult::Err(e.to_string()),
        }
    }
}

#[derive(Debug)]
pub struct FunctionBuilder {
    expression: Option<String>,
    variable_builders: IndexMap<String, Box<dyn SchemaBuilder>>,
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
        builder: impl SchemaBuilder + 'static,
    ) -> Self {
        self.set_variable(key, builder);
        self
    }

    pub fn set_variable(
        &mut self,
        key: impl Into<String>,
        builder: impl SchemaBuilder + 'static,
    ) -> &mut Self {
        self.variable_builders.insert(key.into(), Box::new(builder));
        self
    }
}

impl SchemaBuilder for FunctionBuilder {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
        let expression = match &self.expression {
            Some(e) => e,
            None => return Err(vec![visitor.error("expression was not set")]),
        };

        let mut errors = vec![];
        let mut variables = HashMap::new();

        for (key, builder) in &self.variable_builders {
            let var_visitor = visitor.follow_edge(SchemaEdge {
                kind: "variable",
                key: key.clone(),
            });
            match builder.build(var_visitor) {
                Ok(schema) => {
                    variables.insert(key.clone(), schema);
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

impl SchemaParser for FunctionParser {
    fn should_parse(&self, visitor: &SchemaParseVisitor) -> bool {
        visitor.spec().stype == Some("function".into())
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
        let expression = match visitor.spec().fields.get("expression") {
            Some(v) if v.is_string() => v.as_str().unwrap().to_string(),
            Some(_) => {
                return Err(vec![Error {
                    path: Some(visitor.absolute_sub_path(vec!["expression".into()])),
                    message: "expression must be a string".into(),
                }]);
            }
            None => {
                return Err(vec![Error {
                    path: Some(visitor.absolute_path()),
                    message: "expression must be specified".into(),
                }]);
            }
        };

        let mut builder = Function::builder();
        builder.set_expression(expression);

        if let Some(vars_value) = visitor.spec().fields.get("variables") {
            let vars: IndexMap<String, spec::Schema> = serde_json::from_value(vars_value.clone())
                .map_err(|e| {
                vec![Error {
                    path: Some(visitor.absolute_sub_path(vec!["variables".into()])),
                    message: format!("variables parsing failed: {e}"),
                }]
            })?;

            let mut errors = vec![];
            for (key, schema) in vars {
                let path = vec!["variables".into(), key.clone()];
                match visitor.parse_child(path, schema) {
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

fn json_to_cel(value: serde_json::Value) -> cel::Value {
    match value {
        serde_json::Value::Bool(b) => cel::Value::Bool(b),
        serde_json::Value::Number(n) => {
            if n.is_i64() {
                cel::Value::Int(n.as_i64().unwrap())
            } else if n.is_u64() {
                let safe_i64 = (n.as_u64().unwrap().min(i64::MAX as u64)) as i64;
                cel::Value::Int(safe_i64)
            } else if n.is_f64() {
                cel::Value::Float(n.as_f64().unwrap())
            } else {
                eprintln!("number is not an integer or float: {n}");
                cel::Value::Int(0)
            }
        }
        serde_json::Value::String(s) => cel::Value::String(s.into()),
        serde_json::Value::Array(a) => {
            cel::Value::List(a.into_iter().map(json_to_cel).collect::<Vec<_>>().into())
        }
        serde_json::Value::Object(o) => cel::Value::Map(cel::objects::Map::from(
            o.into_iter()
                .map(|(k, v)| (k, json_to_cel(v)))
                .collect::<HashMap<_, _>>(),
        )),
        serde_json::Value::Null => cel::Value::Null,
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
