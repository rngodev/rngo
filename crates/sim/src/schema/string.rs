use super::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaResult};
use crate::build::BuildError;
use crate::parse::{SchemaParseVisitor, SchemaParser};
use crate::schema::SchemaContext;
use crate::spec::ParseError as Error;
use rand::distr::Distribution;
use rand_pcg::Pcg32;

pub struct Str {
    rng: Pcg32,
    regex: rand_regex::Regex,
}

impl std::fmt::Debug for Str {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Str").finish()
    }
}

impl Str {
    pub fn builder() -> StrBuilder {
        StrBuilder {
            pattern: ".{0,64}".into(),
        }
    }

    pub fn parser() -> StrParser {
        StrParser {}
    }
}

impl Schema for Str {
    fn next(&mut self, _context: &SchemaContext) -> SchemaResult {
        let s: String = self.regex.sample(&mut self.rng);
        SchemaResult::Ok { value: s.into() }
    }
}

#[derive(Debug)]
pub struct StrBuilder {
    pattern: String,
}

impl StrBuilder {
    pub fn pattern(mut self, pattern: impl Into<String>) -> Self {
        self.set_pattern(pattern);
        self
    }

    pub fn set_pattern(&mut self, pattern: impl Into<String>) -> &mut Self {
        self.pattern = pattern.into();
        self
    }
}

impl SchemaBuilder for StrBuilder {
    fn build(&self, visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
        let regex = rand_regex::Regex::compile(&self.pattern, 100)
            .map_err(|e| vec![visitor.error(format!("invalid pattern: {e}"))])?;

        Ok(Box::new(Str {
            rng: visitor.rng(),
            regex,
        }))
    }
}

pub struct StrParser {}

impl SchemaParser for StrParser {
    fn key(&self) -> &str {
        "string"
    }

    fn parse(&self, visitor: SchemaParseVisitor) -> Result<Box<dyn SchemaBuilder>, Vec<Error>> {
        let mut builder = Str::builder();

        match visitor.spec().fields.get("pattern") {
            Some(v) if v.is_string() => {
                builder.set_pattern(v.as_str().unwrap());
                Ok(Box::new(builder))
            }
            Some(_) => Err(vec![
                visitor.input_error("pattern", "pattern must be a string"),
            ]),
            None => Err(vec![visitor.schema_error("pattern must be specified")]),
        }
    }
}
