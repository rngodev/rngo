use crate::signal::{Signal, SignalOutcome};
use crate::system::System;
use indexmap::IndexMap;

/// A named list of [`Signal`]s, built by [`crate::parse::Dialect::parse_audit`] from a spec's
/// `signals`. Decoupled from [`System`]: it's evaluated against one on demand via [`Audit::run`]
/// rather than being carried by the `System` itself.
#[derive(Debug)]
pub struct Audit {
    signals: IndexMap<String, Box<dyn Signal>>,
}

impl Audit {
    pub(crate) fn new(signals: IndexMap<String, Box<dyn Signal>>) -> Self {
        Audit { signals }
    }

    pub fn run(&self, system: &System) -> AuditReport {
        let reader = system.reader();

        let outcomes = self
            .signals
            .iter()
            .map(|(key, signal)| (key.clone(), signal.evaluate(reader.as_ref())))
            .collect();

        AuditReport { outcomes }
    }
}

#[derive(Debug)]
pub struct AuditReport {
    pub outcomes: IndexMap<String, SignalOutcome>,
}
