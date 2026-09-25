use crate::build::BuildError;
pub mod signal;

use crate::run_log::Metadata;
use crate::{RunLogReader, RunLogWriter};
use indexmap::IndexMap;
use signal::{Signal, SignalBuilder, SignalOutcome};
use std::rc::Rc;

#[derive(Debug)]
pub struct Audit {
    signals: IndexMap<String, Box<dyn Signal>>,
    run_log_reader: Rc<dyn RunLogReader>,
    run_log_writer: Rc<dyn RunLogWriter>,
}

impl Audit {
    pub(crate) fn new(
        signals: IndexMap<String, Box<dyn Signal>>,
        run_log_reader: Rc<dyn RunLogReader>,
        run_log_writer: Rc<dyn RunLogWriter>,
    ) -> Self {
        Audit {
            signals,
            run_log_reader,
            run_log_writer,
        }
    }

    pub fn builder() -> AuditBuilder {
        AuditBuilder::new()
    }

    pub fn run(&self) -> AuditReport {
        let outcomes: IndexMap<String, SignalOutcome> = self
            .signals
            .iter()
            .map(|(key, signal)| (key.clone(), signal.evaluate(self.run_log_reader.as_ref())))
            .collect();

        for (key, outcome) in &outcomes {
            let mut data = serde_json::to_value(outcome).unwrap();
            if let serde_json::Value::Object(map) = &mut data {
                map.insert("key".to_string(), serde_json::Value::String(key.clone()));
            }

            self.run_log_writer.push_metadata(Metadata {
                mtype: "signal".to_string(),
                input_id: None,
                output_id: None,
                offset: None,
                data: Some(data),
                segment: None,
            });
        }

        AuditReport { outcomes }
    }
}

pub struct AuditBuilder {
    signal_builders: Vec<(String, Box<dyn SignalBuilder>)>,
    run_log_reader: Option<Rc<dyn RunLogReader>>,
    run_log_writer: Option<Rc<dyn RunLogWriter>>,
}

impl AuditBuilder {
    pub fn new() -> Self {
        AuditBuilder {
            signal_builders: vec![],
            run_log_reader: None,
            run_log_writer: None,
        }
    }

    pub fn with_signal(
        mut self,
        key: impl Into<String>,
        builder: impl SignalBuilder + 'static,
    ) -> Self {
        self.set_signal(key, builder);
        self
    }

    pub fn set_signal(
        &mut self,
        key: impl Into<String>,
        builder: impl SignalBuilder + 'static,
    ) -> &mut Self {
        self.signal_builders.push((key.into(), Box::new(builder)));
        self
    }

    pub fn run_log_reader<T: RunLogReader + 'static>(mut self, reader: Rc<T>) -> Self {
        self.run_log_reader = Some(reader as Rc<dyn RunLogReader>);
        self
    }

    pub fn run_log_writer<T: RunLogWriter + 'static>(mut self, writer: Rc<T>) -> Self {
        self.run_log_writer = Some(writer as Rc<dyn RunLogWriter>);
        self
    }

    /// Convenience for the common case where a single backend serves as both reader and writer -
    /// equivalent to calling [`AuditBuilder::run_log_reader`] and [`AuditBuilder::run_log_writer`]
    /// with clones of the same handle. Reach for those directly if the reader and writer need to
    /// be different objects.
    pub fn run_log<T: RunLogReader + RunLogWriter + 'static>(self, run_log: Rc<T>) -> Self {
        self.run_log_reader(run_log.clone()).run_log_writer(run_log)
    }

    pub fn build(self) -> Result<Audit, Vec<BuildError>> {
        let mut errors = vec![];
        let mut signals = IndexMap::new();

        for (key, builder) in self.signal_builders {
            match builder.build(&key) {
                Ok(signal) => {
                    signals.insert(key, signal);
                }
                Err(mut e) => errors.append(&mut e),
            }
        }

        // Unlike `SimulationBuilder`, which writes the run it produces to whatever run log it's
        // given (or a throwaway default), an `Audit` only ever reads and annotates a run log some
        // other process already populated - defaulting here would silently evaluate every signal
        // against an empty log instead of surfacing the missing wiring.
        let run_log = match (self.run_log_reader, self.run_log_writer) {
            (Some(reader), Some(writer)) => Some((reader, writer)),
            _ => {
                errors.push(BuildError::Audit {
                    message: "run_log was not set".into(),
                });
                None
            }
        };

        if !errors.is_empty() {
            return Err(errors);
        }

        let (run_log_reader, run_log_writer) = run_log.expect("checked above");
        Ok(Audit::new(signals, run_log_reader, run_log_writer))
    }
}

impl Default for AuditBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
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
    use crate::{Input, RunLogReader, RunLogWriter};
    use signal::SignalEval;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Debug)]
    struct MockSignal(SignalOutcome);

    impl Signal for MockSignal {
        fn evaluate(&self, _run_log: &dyn RunLogReader) -> SignalOutcome {
            self.0.clone()
        }
    }

    /// A run log whose only job is to capture every `Metadata` row pushed to it, so a test can
    /// assert on exactly what `Audit::run` writes back - `SimpleEventRunLog` has no way to read
    /// its metadata back out, and `SqliteRunLog` needs a real directory on disk.
    #[derive(Debug, Default)]
    struct MockRunLog {
        metadata: RefCell<Vec<Metadata>>,
    }

    impl RunLogReader for MockRunLog {
        fn last(&self) -> Option<Rc<Input>> {
            None
        }

        fn last_for_effect(&self, _key: &str) -> Option<Rc<Input>> {
            None
        }

        fn query(&self, _query: &str) -> Option<serde_json::Value> {
            None
        }

        fn random_for_effect(&self, _key: &str, _rng: &mut rand_pcg::Pcg32) -> Option<Rc<Input>> {
            unimplemented!("not exercised by these tests")
        }

        fn unique_for_effect(
            &self,
            _key: &str,
            _cursor: &str,
            _rng: &mut rand_pcg::Pcg32,
        ) -> Option<Rc<Input>> {
            unimplemented!("not exercised by these tests")
        }
    }

    impl RunLogWriter for MockRunLog {
        fn push_input(&self, _input: Input) {
            unimplemented!("not exercised by these tests")
        }

        fn push_output(&self, _output: crate::Output) {
            unimplemented!("not exercised by these tests")
        }

        fn push_metadata(&self, metadata: Metadata) {
            self.metadata.borrow_mut().push(metadata);
        }
    }

    fn success(passed: bool) -> SignalOutcome {
        SignalOutcome::Success {
            value: serde_json::json!(1),
            eval: Some(SignalEval {
                expectation: "result == 1".to_string(),
                passed,
            }),
        }
    }

    fn success_without_expect() -> SignalOutcome {
        SignalOutcome::Success {
            value: serde_json::json!(1),
            eval: None,
        }
    }

    fn error() -> SignalOutcome {
        SignalOutcome::Error {
            error: "boom".to_string(),
        }
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

        let run_log = Rc::new(MockRunLog::default());
        let audit = Audit::new(signals, run_log.clone(), run_log);

        let audit_report = audit.run();

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

        let run_log = Rc::new(MockRunLog::default());
        let audit = Audit::new(signals, run_log.clone(), run_log.clone());

        audit.run();

        let written = run_log.metadata.borrow();
        assert_eq!(written.len(), 2);

        assert_eq!(written[0].mtype, "signal");
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

        assert_eq!(written[1].mtype, "signal");
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

        let run_log = Rc::new(MockRunLog::default());
        let audit = Audit::builder()
            .with_signal(
                "check",
                sql_signal().query("SELECT 1").expect("result == 1"),
            )
            .run_log(run_log)
            .build()
            .unwrap();

        let report = audit.run();

        assert_eq!(report.outcomes.keys().collect::<Vec<_>>(), vec!["check"]);
    }

    #[test]
    fn builder_without_a_run_log_is_an_error() {
        use crate::build::sql_signal;

        let result = Audit::builder()
            .with_signal("check", sql_signal().query("SELECT 1"))
            .build();

        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn builder_collects_errors_from_every_failing_signal_builder() {
        use crate::build::sql_signal;

        let run_log = Rc::new(MockRunLog::default());
        let result = Audit::builder()
            .with_signal("no-query", sql_signal())
            .with_signal("also-no-query", sql_signal())
            .run_log(run_log)
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
