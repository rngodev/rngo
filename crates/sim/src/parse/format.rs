use crate::format::Format;
use crate::spec::{self, ParseError};

pub trait FormatParser {
    fn should_parse(&self, context: &FormatParseContext) -> bool;
    fn parse(&self, context: FormatParseContext) -> Result<Box<dyn Format>, Vec<ParseError>>;
}

pub struct FormatParseContext {
    simulation: spec::Simulation,
    effect_key: String,
}

impl FormatParseContext {
    pub fn new(simulation: spec::Simulation, effect_key: String) -> Result<Self, ParseError> {
        let effect = simulation
            .effects
            .get(&effect_key)
            .unwrap_or_else(|| panic!("expected effect at key {effect_key}"));

        let effect_ftype = effect.format.as_ref().and_then(|f| f.ftype.as_deref());
        let system_ftype = effect
            .system
            .as_ref()
            .and_then(|s| simulation.systems.get(s))
            .and_then(|s| s.format.as_ref())
            .and_then(|f| f.ftype.as_ref());

        if let (Some(ef), Some(sf)) = (effect_ftype, system_ftype)
            && ef != sf
        {
            return Err(ParseError::SchemaError {
                path: Some(vec![
                    "effects".into(),
                    effect_key.clone(),
                    "format".into(),
                    "type".into(),
                ]),
                message: format!(
                    "effect format type \"{ef}\" does not match system format type \"{sf}\""
                ),
            });
        }

        Ok(FormatParseContext {
            simulation,
            effect_key,
        })
    }

    pub fn effect(&self) -> &spec::Effect {
        self.simulation
            .effects
            .get(&self.effect_key)
            .unwrap_or_else(|| panic!("expected effect at key {}", self.effect_key))
    }

    pub fn effect_key(&self) -> &str {
        &self.effect_key
    }

    pub fn is_format_type(&self, ftype: &str) -> bool {
        self.format()
            .and_then(|f| f.ftype)
            .map(|ft| ft == ftype)
            .unwrap_or(false)
    }

    pub fn format(&self) -> Option<spec::Format> {
        let effect = self.effect();

        let system_format = effect
            .system
            .as_ref()
            .and_then(|s| self.simulation.systems.get(s))
            .and_then(|s| s.format.as_ref());

        match (effect.format.as_ref(), system_format) {
            (Some(ef), Some(sf)) => {
                let mut merged = sf.clone();
                if ef.ftype.is_some() {
                    merged.ftype = ef.ftype.clone();
                }
                for (k, v) in &ef.fields {
                    merged.fields.insert(k.clone(), v.clone());
                }
                Some(merged)
            }
            (Some(ef), None) => Some(ef.clone()),
            (None, Some(sf)) => Some(sf.clone()),
            (None, None) => None,
        }
    }
}
