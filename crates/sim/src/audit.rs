use crate::run_log::EffectMetadata;
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

    /// Evaluates every signal against `system`'s run log, then logs each outcome back into it as
    /// its own `metadata` row (`data.key` carries the signal's key, since a signal has no
    /// associated input and the table has no `effect` column) - the same log the run itself wrote
    /// its inputs/outputs/metadata to.
    pub fn run(&self, system: &mut System) -> AuditReport {
        let reader = system.reader();

        let outcomes: IndexMap<String, SignalOutcome> = self
            .signals
            .iter()
            .map(|(key, signal)| (key.clone(), signal.evaluate(reader.as_ref())))
            .collect();

        for (key, outcome) in &outcomes {
            let mut data = serde_json::to_value(outcome).unwrap();
            if let serde_json::Value::Object(map) = &mut data {
                map.insert("key".to_string(), serde_json::Value::String(key.clone()));
            }

            system.add_metadata(EffectMetadata {
                mtype: "signal".to_string(),
                input_id: None,
                output_id: None,
                offset: None,
                attribute: None,
                data: Some(data),
                segment: None,
            });
        }

        AuditReport { outcomes }
    }
}

#[derive(Debug)]
pub struct AuditReport {
    pub outcomes: IndexMap<String, SignalOutcome>,
}
