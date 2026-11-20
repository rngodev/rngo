use super::{Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorResult};
use crate::effect::generator::{GeneratorContext, GeneratorParseVisitor, GeneratorParser};
use crate::error::{BuildErrors, ParseErrors};
use rand::distr::Distribution;
use rand_pcg::Pcg32;

pub(crate) struct Str {
    rng: Pcg32,
    regex: rand_regex::Regex,
}

impl std::fmt::Debug for Str {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Str").finish()
    }
}

impl Str {
    pub(crate) fn builder() -> StrBuilder {
        StrBuilder {
            pattern: ".{0,64}".into(),
        }
    }

    pub(crate) fn parser() -> StrParser {
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
        self.pattern = pattern.into();
        self
    }
}

impl GeneratorBuilder for StrBuilder {
    fn build(&self, visitor: GeneratorBuildVisitor) -> Result<Box<dyn Generator>, BuildErrors> {
        let regex = rand_regex::Regex::compile(&self.pattern, 100)
            .map_err(|e| vec![visitor.error(format!("invalid pattern: {e}"))])?;

        Ok(Box::new(Str {
            rng: visitor.rng().0,
            regex,
        }))
    }
}

pub(crate) struct StrParser {}

impl GeneratorParser for StrParser {
    fn key(&self) -> &str {
        "string"
    }

    fn parse(
        &self,
        visitor: GeneratorParseVisitor,
    ) -> Result<Box<dyn GeneratorBuilder>, ParseErrors> {
        let mut builder = Str::builder();

        match visitor.spec().fields.get("pattern").map(|v| v.as_str()) {
            Some(Some(pattern)) => {
                builder = builder.pattern(pattern);
                Ok(Box::new(builder))
            }
            Some(_) => Err(visitor
                .field_error("pattern", "pattern must be a string")
                .into()),
            None => Err(visitor.error("pattern must be specified").into()),
        }
    }
}
