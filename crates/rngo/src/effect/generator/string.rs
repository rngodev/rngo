use super::{Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorResult};
use crate::build::BuildError;
use crate::effect::generator::GeneratorContext;
use crate::parse::{GeneratorParseVisitor, GeneratorParser};
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

impl Generator for Str {
    fn next(&mut self, _context: &GeneratorContext) -> GeneratorResult {
        let s: String = self.regex.sample(&mut self.rng);
        GeneratorResult {
            value: Some(s.into()),
            metadata: vec![],
        }
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

impl GeneratorBuilder for StrBuilder {
    fn build(&self, visitor: GeneratorBuildVisitor) -> Result<Box<dyn Generator>, Vec<BuildError>> {
        let regex = rand_regex::Regex::compile(&self.pattern, 100)
            .map_err(|e| vec![visitor.error(format!("invalid pattern: {e}"))])?;

        Ok(Box::new(Str {
            rng: visitor.rng(),
            regex,
        }))
    }
}

pub struct StrParser {}

impl GeneratorParser for StrParser {
    fn key(&self) -> &str {
        "string"
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, Vec<Error>> {
        let mut builder = Str::builder();

        match visitor.spec().fields.get("pattern") {
            Some(v) if v.is_string() => {
                builder.set_pattern(v.as_str().unwrap());
                Ok(Box::new(builder))
            }
            Some(_) => Err(vec![
                visitor.input_error("pattern", "pattern must be a string"),
            ]),
            None => Err(vec![visitor.invalid("pattern must be specified")]),
        }
    }
}
