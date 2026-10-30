use crate::build::{BuildError, SimulationKey};
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
    parent: Option<Phase>,
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

    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.end = Some(end);
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

    /// Resolves the phase, taking relative moments from `now`; errors if `start` isn't before `end`.
    pub fn build(&self, now: DateTime<FixedOffset>) -> Result<Phase, Vec<BuildError>> {
        let default = Phase::default_at(now);
        let parent = self.parent.as_ref().unwrap_or(&default);
        let start = self
            .start
            .clone()
            .map_or(parent.start, |start| start.resolve(now));
        let end = self.end.clone().map_or(parent.end, |end| end.resolve(now));

        if start >= end {
            let message = match &self.name {
                Some(name) => format!("phase \"{name}\": start must be before end"),
                None => "start must be before end".into(),
            };
            return Err(vec![BuildError::simulation(SimulationKey::Start, message)]);
        }

        Ok(Phase {
            name: self.name.clone(),
            start,
            end,
        })
    }
}
