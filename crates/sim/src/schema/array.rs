use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult};
use crate::build::{BuildError, SchemaEdge};
use crate::spec::{self, ParseError as Error, SchemaParseVisitor, SchemaParser};
use rand::RngExt;
use rand_pcg::Pcg32;

#[derive(Debug)]
pub struct Array {
    rng: Pcg32,
    min_items: usize,
    max_items: usize,
    items: Box<dyn Schema>,
}

impl Array {
    pub fn builder() -> ArrayBuilder {
        ArrayBuilder {
            min_items: 0,
            max_items: usize::MAX,
            items_builder: None,
        }
    }

    pub fn parser() -> ArrayParser {
        ArrayParser {}
    }
}

#[derive(Debug)]
pub struct ArrayBuilder {
    min_items: usize,
    max_items: usize,
    items_builder: Option<Box<dyn SchemaBuilder>>,
}

impl ArrayBuilder {
    pub fn min_items(mut self, value: usize) -> Self {
        self.set_min_items(value);
        self
    }

    pub fn set_min_items(&mut self, value: usize) -> &mut Self {
        self.min_items = value;
        self
    }

    pub fn max_items(mut self, value: usize) -> Self {
        self.set_max_items(value);
        self
    }

    pub fn set_max_items(&mut self, value: usize) -> &mut Self {
        self.max_items = value;
        self
    }

    pub fn items(mut self, builder: impl SchemaBuilder + 'static) -> Self {
        self.set_items(builder);
        self
    }

    pub fn set_items(&mut self, builder: impl SchemaBuilder + 'static) -> &mut Self {
        self.items_builder = Some(Box::new(builder));
        self
    }
}

impl SchemaBuilder for ArrayBuilder {
    fn build(
        &self,
        schema_visitor: SchemaBuildVisitor,
    ) -> Result<Box<dyn Schema>, Vec<BuildError>> {
        let items = if let Some(items_builder) = &self.items_builder {
            items_builder.build(schema_visitor.follow_edge(SchemaEdge {
                kind: "items",
                key: "items".into(),
            }))?
        } else {
            return Err(vec![schema_visitor.error("items was not set")]);
        };

        Ok(Box::new(Array {
            rng: schema_visitor.rng(),
            min_items: self.min_items,
            max_items: self.max_items,
            items,
        }))
    }
}

impl Schema for Array {
    fn next(&mut self, context: &SchemaContext) -> SchemaResult {
        let count = self.rng.random_range(self.min_items..=self.max_items);
        let mut arr = Vec::with_capacity(count);

        for _ in 0..count {
            match self.items.next(context) {
                SchemaResult::Ok { value } => arr.push(value),
                SchemaResult::Err(e) => return SchemaResult::Err(e),
            }
        }

        SchemaResult::Ok { value: arr.into() }
    }
}

pub struct ArrayParser {}

impl SchemaParser for ArrayParser {
    fn key(&self) -> &str {
        "array"
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
        let items_spec = match visitor.spec().fields.get("items") {
            Some(v) => serde_json::from_value::<spec::Schema>(v.clone()).map_err(|e| {
                vec![visitor.input_error("items", format!("items parsing failed: {e}"))]
            })?,
            None => {
                return Err(vec![visitor.schema_error("items must be specified")]);
            }
        };

        let items_builder = visitor.parse_child(vec!["items".into()], items_spec)?;

        let mut errors = vec![];
        let mut min_items = 0usize;
        let mut max_items = usize::MAX;

        if let Some(v) = visitor.spec().fields.get("minItems") {
            match v.as_u64() {
                Some(n) => min_items = n as usize,
                None => errors.push(
                    visitor.input_error("minItems", "minItems must be a non-negative integer"),
                ),
            }
        }

        if let Some(v) = visitor.spec().fields.get("maxItems") {
            match v.as_u64() {
                Some(n) => max_items = n as usize,
                None => errors.push(
                    visitor.input_error("maxItems", "maxItems must be a non-negative integer"),
                ),
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(Box::new(
            Array::builder()
                .min_items(min_items)
                .max_items(max_items)
                .items(items_builder),
        ))
    }
}
