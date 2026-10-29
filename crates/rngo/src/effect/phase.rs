use crate::moment::Moment;
use chrono::TimeDelta;

/// A time window with its `start` and `end` resolved against any parent phase.
#[derive(Clone, Debug)]
pub struct Phase {
    pub start: Moment,
    pub end: Moment,
}

impl Phase {
    pub fn builder() -> PhaseBuilder {
        PhaseBuilder::default()
    }
}

/// Builds a [`Phase`]; a `start` or `end` left unset comes from the parent, then from the built-in default (30 days before now to now).
#[derive(Clone, Debug, Default)]
pub struct PhaseBuilder {
    start: Option<Moment>,
    end: Option<Moment>,
    parent: Option<Phase>,
}

impl PhaseBuilder {
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

    pub fn build(&self) -> Phase {
        let parent = self.parent.as_ref();
        Phase {
            start: self
                .start
                .clone()
                .or_else(|| parent.map(|p| p.start.clone()))
                .unwrap_or(Moment::Relative(TimeDelta::days(-30))),
            end: self
                .end
                .clone()
                .or_else(|| parent.map(|p| p.end.clone()))
                .unwrap_or(Moment::Relative(TimeDelta::zero())),
        }
    }
}
