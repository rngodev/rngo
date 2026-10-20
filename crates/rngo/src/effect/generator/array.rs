use super::{
    Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext, GeneratorResult,
    JsonPointerPart,
};
use crate::build::{BuildError, GeneratorEdge};
use crate::effect::generator::Metadata;
use crate::parse::{GeneratorParseVisitor, GeneratorParser};
use crate::spec::{self, ParseError as Error};
use rand::RngExt;
use rand_pcg::Pcg32;

#[derive(Debug)]
pub struct Array {
    rng: Pcg32,
    min_items: usize,
    max_items: usize,
    items: Box<dyn Generator>,
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
    items_builder: Option<Box<dyn GeneratorBuilder>>,
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

    pub fn items(mut self, builder: impl GeneratorBuilder + 'static) -> Self {
        self.set_items(builder);
        self
    }

    pub fn set_items(&mut self, builder: impl GeneratorBuilder + 'static) -> &mut Self {
        self.items_builder = Some(Box::new(builder));
        self
    }
}

impl GeneratorBuilder for ArrayBuilder {
    fn build(
        &self,
        generator_visitor: GeneratorBuildVisitor,
    ) -> Result<Box<dyn Generator>, Vec<BuildError>> {
        let items = if let Some(items_builder) = &self.items_builder {
            items_builder.build(generator_visitor.follow_edge(GeneratorEdge {
                kind: "items",
                key: "items".into(),
            }))?
        } else {
            return Err(vec![generator_visitor.error("items was not set")]);
        };

        Ok(Box::new(Array {
            rng: generator_visitor.rng(),
            min_items: self.min_items,
            max_items: self.max_items,
            items,
        }))
    }
}

impl Generator for Array {
    fn next(&mut self, context: &GeneratorContext) -> GeneratorResult {
        let count = self.rng.random_range(self.min_items..=self.max_items);
        let mut arr = Vec::with_capacity(count);
        let mut complete = true;
        let mut metadata: Vec<Metadata> = Vec::new();

        for i in 0..count {
            let result = self.items.next(context);

            if let Some(value) = result.value {
                arr.push(value);
            } else {
                complete = false;
            }

            for mut result_metadata in result.metadata {
                result_metadata.prefix_attribute(JsonPointerPart::Index(i as u32));
                metadata.push(result_metadata)
            }
        }

        let value = if complete { Some(arr.into()) } else { None };

        GeneratorResult { value, metadata }
    }
}

pub struct ArrayParser {}

impl GeneratorParser for ArrayParser {
    fn key(&self) -> &str {
        "array"
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<Error>> {
        let items_spec = match visitor.spec().fields.get("items") {
            Some(v) => serde_json::from_value::<spec::Generator>(v.clone()).map_err(|e| {
                vec![visitor.input_error("items", format!("items parsing failed: {e}"))]
            })?,
            None => {
                return Err(vec![visitor.invalid("items must be specified")]);
            }
        };

        let items_builder = visitor.parse_input_generator(vec!["items".into()], items_spec)?;

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
