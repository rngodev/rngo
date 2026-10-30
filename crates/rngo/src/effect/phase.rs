use crate::build::{BuildError, EffectKey, SimulationKey};
use crate::moment::Moment;
use chrono::{DateTime, FixedOffset, TimeDelta};

/// A time window with its `start` and `end` resolved to instants.
#[derive(Clone, Debug)]
pub struct Phase {
    /// The phase's name; `None` for the default phase.
    pub name: Option<String>,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
}

impl Phase {
    pub fn builder() -> PhaseBuilder {
        PhaseBuilder::default()
    }

    /// The built-in window, from 30 days before `now` to `now`.
    pub fn default_at(now: DateTime<FixedOffset>) -> Phase {
        Phase {
            name: None,
            start: now - TimeDelta::days(30),
            end: now,
        }
    }
}

/// Builds a [`Phase`]; a `start` or `end` left unset comes from the parent, then from the built-in default (30 days before now to now).
#[derive(Clone, Debug, Default)]
pub struct PhaseBuilder {
    name: Option<String>,
    start: Option<Moment>,
    end: Option<Moment>,
    start_expression: Option<String>,
    end_expression: Option<String>,
    parent: Option<Phase>,
    effect: Option<String>,
}

impl PhaseBuilder {
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn start(mut self, start: Moment) -> Self {
        self.set_start(start);
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.start = Some(start);
        self
    }

    /// Sets `start` from an expression, parsed at build with the parent's bounds as `simulation`; overrides `start`.
    pub fn set_start_expression(&mut self, expression: String) -> &mut Self {
        self.start_expression = Some(expression);
        self
    }

    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.end = Some(end);
        self
    }

    /// Sets `end` from an expression, parsed at build with the parent's bounds as `simulation`; overrides `end`.
    pub fn set_end_expression(&mut self, expression: String) -> &mut Self {
        self.end_expression = Some(expression);
        self
    }

    /// Makes this the window of effect `key`: it must lie within the parent, and errors are reported against the effect.
    pub fn for_effect(mut self, key: impl Into<String>) -> Self {
        self.effect = Some(key.into());
        self
    }

    pub fn parent(mut self, parent: &Phase) -> Self {
        self.set_parent(parent);
        self
    }

    /// Sets the phase that supplies whichever of `start` and `end` this one leaves unset.
    pub fn set_parent(&mut self, parent: &Phase) -> &mut Self {
        self.parent = Some(parent.clone());
        self
    }

    /// Resolves the phase, taking relative moments from `now`; errors if an expression doesn't parse, and otherwise if `start` isn't before `end` (or, for an effect, if the window leaves the parent).
    pub fn build(&self, now: DateTime<FixedOffset>) -> Result<Phase, Vec<BuildError>> {
        let default = Phase::default_at(now);
        let parent = self.parent.as_ref().unwrap_or(&default);
        let parent_start = Moment::from(parent.start);
        let parent_end = Moment::from(parent.end);
        let parser = Moment::parser().simulation(&parent_start, &parent_end);
        let mut errors = vec![];

        let mut bound = |field: &str, moment: &Option<Moment>, expression: &Option<String>| {
            let moment = match expression {
                Some(expression) => match parser.parse(field, expression) {
                    Ok(moment) => Some(moment),
                    Err(e) => {
                        errors.extend(e.iter().map(|e| self.error(field, e.message())));
                        return None;
                    }
                },
                None => moment.clone(),
            };
            Some(moment.map(|moment| moment.resolve(now)))
        };
        let start = bound("start", &self.start, &self.start_expression);
        let end = bound("end", &self.end, &self.end_expression);
        let (Some(start), Some(end)) = (start, end) else {
            return Err(errors);
        };
        let start = start.unwrap_or(parent.start);
        let end = end.unwrap_or(parent.end);

        if self.effect.is_some() {
            let bounds = match &parent.name {
                Some(name) => format!("phase \"{name}\""),
                None => "simulation".into(),
            };
            if start < parent.start {
                errors.push(self.error("start", format!("start cannot be before {bounds} start")));
            }
            if end > parent.end {
                errors.push(self.error("end", format!("end cannot be after {bounds} end")));
            }
        } else if start >= end {
            let message = match &self.name {
                Some(name) => format!("phase \"{name}\": start must be before end"),
                None => "start must be before end".into(),
            };
            errors.push(self.error("start", message));
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(Phase {
            name: self.name.clone(),
            start,
            end,
        })
    }

    fn error(&self, field: &str, message: impl Into<String>) -> BuildError {
        match &self.effect {
            Some(key) => {
                let key_kind = if field == "start" {
                    EffectKey::Start
                } else {
                    EffectKey::End
                };
                BuildError::effect(key.clone(), key_kind, message)
            }
            None => BuildError::simulation(SimulationKey::Start, message),
        }
    }
}
