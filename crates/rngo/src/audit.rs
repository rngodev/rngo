use crate::error::{BuildError, BuildErrors};
use crate::log::Metadata;
pub mod signal;

use crate::{LogError, LogReader, LogWriter};
use indexmap::IndexMap;
use signal::{Signal, SignalBuildContext, SignalBuilder, SignalContext, SignalOutcome};
use std::sync::Arc;

#[derive(Debug)]
pub struct Audit {
    signals: IndexMap<String, Box<dyn Signal>>,
    log_reader: Arc<dyn LogReader>,
    log_writer: Arc<dyn LogWriter>,
}

impl Audit {
    pub(crate) fn new(
        signals: IndexMap<String, Box<dyn Signal>>,
        log_reader: Arc<dyn LogReader>,
        log_writer: Arc<dyn LogWriter>,
    ) -> Self {
        Audit {
            signals,
            log_reader,
            log_writer,
        }
    }

    pub fn builder() -> AuditBuilder {
        AuditBuilder::new()
    }

    /// Evaluates every signal and writes each outcome to the log as metadata.
    pub fn run(&self) -> Result<AuditReport, LogError> {
        let context = SignalContext::new(self.log_reader.as_ref());
        let outcomes: IndexMap<String, SignalOutcome> = self
            .signals
            .iter()
            .map(|(key, signal)| (key.clone(), signal.evaluate(&context)))
            .collect();

        for (key, outcome) in &outcomes {
            let mut data = serde_json::to_value(outcome)?;
            if let serde_json::Value::Object(map) = &mut data {
                map.insert("key".to_string(), serde_json::Value::String(key.clone()));
            }

            self.log_writer
                .push_metadata(Metadata::new("signal").data(data))?;
        }

        Ok(AuditReport { outcomes })
    }
}

pub struct AuditBuilder {
    signal_builders: Vec<(String, Box<dyn SignalBuilder>)>,
    log_reader: Option<Arc<dyn LogReader>>,
    log_writer: Option<Arc<dyn LogWriter>>,
}

impl AuditBuilder {
    pub fn new() -> Self {
        AuditBuilder {
            signal_builders: vec![],
            log_reader: None,
            log_writer: None,
        }
    }

    /// Adds a signal under `key`.
    pub fn signal(mut self, key: impl Into<String>, builder: impl SignalBuilder + 'static) -> Self {
        self.signal_builders.push((key.into(), Box::new(builder)));
        self
    }

    pub fn log_reader<T: LogReader + 'static>(mut self, reader: Arc<T>) -> Self {
        self.log_reader = Some(reader as Arc<dyn LogReader>);
        self
    }

    pub fn log_writer<T: LogWriter + 'static>(mut self, writer: Arc<T>) -> Self {
        self.log_writer = Some(writer as Arc<dyn LogWriter>);
        self
    }

    /// Convenience for the common case where a single backend serves as both reader and writer -
    /// equivalent to calling [`AuditBuilder::log_reader`] and [`AuditBuilder::log_writer`]
    /// with clones of the same handle. Reach for those directly if the reader and writer need to
    /// be different objects.
    pub fn log<T: LogReader + LogWriter + 'static>(self, log: Arc<T>) -> Self {
        self.log_reader(log.clone()).log_writer(log)
    }

    pub fn build(self) -> Result<Audit, BuildErrors> {
        let mut errors = vec![];
        let mut signals = IndexMap::new();

        for (key, builder) in self.signal_builders {
            match builder.build(SignalBuildContext::new(&key)) {
                Ok(signal) => {
                    signals.insert(key, signal);
                }
                Err(e) => errors.extend(e),
            }
        }

        // Unlike `SimulationBuilder`, which writes the run it produces to whatever log it's
        // given (or a throwaway default), an `Audit` only ever reads and annotates a log some
        // other process already populated - defaulting here would silently evaluate every signal
        // against an empty log instead of surfacing the missing wiring.
        let log = match (self.log_reader, self.log_writer) {
            (Some(reader), Some(writer)) => Some((reader, writer)),
            _ => {
                errors.push(BuildError::audit("log was not set"));
                None
            }
        };

        if !errors.is_empty() {
            return Err(errors.into());
        }

        let (log_reader, log_writer) = log.expect("checked above");
        Ok(Audit::new(signals, log_reader, log_writer))
    }
}

impl Default for AuditBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Every signal's outcome, by signal key.
#[derive(Debug)]
#[non_exhaustive]
pub struct AuditReport {
    pub outcomes: IndexMap<String, SignalOutcome>,
}

impl AuditReport {
    pub fn passed(&self) -> bool {
        self.outcomes
            .iter()
            .filter(|(_, outcome)| match outcome {
                SignalOutcome::Success { eval, .. } => {
                    !eval.as_ref().map(|e| e.passed).unwrap_or(true)
                }
                SignalOutcome::Error { .. } => true,
            })
            .count()
            == 0
    }

    pub fn eval_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, outcome)| match outcome {
                SignalOutcome::Success { eval, .. } => eval.is_some(),
                _ => false,
            })
            .count()
    }

    pub fn pass_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, outcome)| match outcome {
                SignalOutcome::Success { eval, .. } => {
                    eval.as_ref().map(|e| e.passed).unwrap_or(false)
                }
                _ => false,
            })
            .count()
    }

    pub fn fail_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, outcome)| match outcome {
                SignalOutcome::Success { eval, .. } => {
                    !eval.as_ref().map(|e| e.passed).unwrap_or(true)
                }
                _ => false,
            })
            .count()
    }

    pub fn error_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, outcome)| matches!(outcome, SignalOutcome::Error { .. }))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Input, LogReader, LogWriter};
    use signal::SignalEval;
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    struct MockSignal(SignalOutcome);

    impl Signal for MockSignal {
        fn evaluate(&self, _context: &SignalContext) -> SignalOutcome {
            self.0.clone()
        }
    }

    /// A log whose only job is to capture every `Metadata` row pushed to it, so a test can
    /// assert on exactly what `Audit::run` writes back - `SimpleLog` has no way to read
    /// its metadata back out, and `SqliteLog` needs a real directory on disk.
    #[derive(Debug, Default)]
    struct MockLog {
        metadata: Mutex<Vec<Metadata>>,
    }

    impl LogReader for MockLog {
        fn last(&self) -> Option<Arc<Input>> {
            None
        }

        fn last_for_effect(&self, _key: &str) -> Option<Arc<Input>> {
            None
        }

        fn query(&self, _query: &str) -> Result<serde_json::Value, LogError> {
            Err(LogError::Unsupported)
        }

        fn random_for_effect(&self, _key: &str, _rng: &mut crate::Rng) -> Option<Arc<Input>> {
            unimplemented!("not exercised by these tests")
        }

        fn unique_for_effect(
            &self,
            _key: &str,
            _cursor: &str,
            _rng: &mut crate::Rng,
        ) -> Option<Arc<Input>> {
            unimplemented!("not exercised by these tests")
        }
    }

    impl LogWriter for MockLog {
        fn push_input(&self, _input: Input) -> Result<(), LogError> {
            unimplemented!("not exercised by these tests")
        }

        fn push_output(&self, _output: crate::Output) -> Result<(), LogError> {
            unimplemented!("not exercised by these tests")
        }

        fn push_metadata(&self, metadata: Metadata) -> Result<(), LogError> {
            self.metadata.lock().unwrap().push(metadata);
            Ok(())
        }
    }

    fn success(passed: bool) -> SignalOutcome {
        SignalOutcome::success(
            serde_json::json!(1),
            Some(SignalEval::new("result == 1", passed)),
        )
    }

    fn success_without_expect() -> SignalOutcome {
        SignalOutcome::success(serde_json::json!(1), None)
    }

    fn error() -> SignalOutcome {
        SignalOutcome::error("boom")
    }

    fn report(outcomes: Vec<(&str, SignalOutcome)>) -> AuditReport {
        AuditReport {
            outcomes: outcomes
                .into_iter()
                .map(|(key, outcome)| (key.to_string(), outcome))
                .collect(),
        }
    }

    #[test]
    fn run_evaluates_every_signal_and_preserves_key_order() {
        let mut signals: IndexMap<String, Box<dyn Signal>> = IndexMap::new();
        signals.insert("b".to_string(), Box::new(MockSignal(success(true))));
        signals.insert("a".to_string(), Box::new(MockSignal(success(false))));

        let log = Arc::new(MockLog::default());
        let audit = Audit::new(signals, log.clone(), log);

        let audit_report = audit.run().unwrap();

        let keys: Vec<_> = audit_report.outcomes.keys().collect();
        assert_eq!(keys, vec!["b", "a"]);
    }

    #[test]
    fn run_writes_one_metadata_row_per_signal_with_its_key() {
        let mut signals: IndexMap<String, Box<dyn Signal>> = IndexMap::new();
        signals.insert(
            "has-events".to_string(),
            Box::new(MockSignal(success(true))),
        );
        signals.insert("errored".to_string(), Box::new(MockSignal(error())));

        let log = Arc::new(MockLog::default());
        let audit = Audit::new(signals, log.clone(), log.clone());

        audit.run().unwrap();

        let written = log.metadata.lock().unwrap();
        assert_eq!(written.len(), 2);

        assert_eq!(written[0].kind, "signal");
        assert_eq!(written[0].input_id, None);
        assert_eq!(written[0].output_id, None);
        assert_eq!(
            written[0].data.as_ref().unwrap()["key"],
            serde_json::json!("has-events")
        );
        assert_eq!(
            written[0].data.as_ref().unwrap()["status"],
            serde_json::json!("success")
        );

        assert_eq!(written[1].kind, "signal");
        assert_eq!(
            written[1].data.as_ref().unwrap()["key"],
            serde_json::json!("errored")
        );
        assert_eq!(
            written[1].data.as_ref().unwrap()["status"],
            serde_json::json!("error")
        );
    }

    #[test]
    fn builder_wires_signal_builders_into_a_runnable_audit() {
        use crate::build::sql_signal;

        let log = Arc::new(MockLog::default());
        let audit = Audit::builder()
            .signal(
                "check",
                sql_signal().query("SELECT 1").expect("result == 1"),
            )
            .log(log)
            .build()
            .unwrap();

        let report = audit.run().unwrap();

        assert_eq!(report.outcomes.keys().collect::<Vec<_>>(), vec!["check"]);
    }

    #[test]
    fn builder_without_a_log_is_an_error() {
        use crate::build::sql_signal;

        let result = Audit::builder()
            .signal("check", sql_signal().query("SELECT 1"))
            .build();

        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn builder_collects_errors_from_every_failing_signal_builder() {
        use crate::build::sql_signal;

        let log = Arc::new(MockLog::default());
        let result = Audit::builder()
            .signal("no-query", sql_signal())
            .signal("also-no-query", sql_signal())
            .log(log)
            .build();

        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn passed_is_true_when_every_eval_passes_and_nothing_errors() {
        let report = report(vec![("a", success(true)), ("b", success_without_expect())]);
        assert!(report.passed());
    }

    #[test]
    fn passed_is_false_when_any_eval_fails() {
        let report = report(vec![("a", success(true)), ("b", success(false))]);
        assert!(!report.passed());
    }

    #[test]
    fn passed_is_false_when_any_signal_errors() {
        let report = report(vec![("a", success(true)), ("b", error())]);
        assert!(!report.passed());
    }

    #[test]
    fn counts_distinguish_evaluated_passed_failed_and_errored() {
        let report = report(vec![
            ("has-events", success(true)),
            ("too-many", success(false)),
            ("no-expect", success_without_expect()),
            ("broken", error()),
        ]);

        assert_eq!(report.eval_count(), 2);
        assert_eq!(report.pass_count(), 1);
        assert_eq!(report.fail_count(), 1);
        assert_eq!(report.error_count(), 1);
        assert!(!report.passed());
    }

    #[test]
    fn empty_report_has_passed_and_zero_counts() {
        let report = report(vec![]);

        assert!(report.passed());
        assert_eq!(report.eval_count(), 0);
        assert_eq!(report.pass_count(), 0);
        assert_eq!(report.fail_count(), 0);
        assert_eq!(report.error_count(), 0);
    }
}
