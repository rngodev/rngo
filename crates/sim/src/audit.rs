use crate::RunLog;
use crate::run_log::Metadata;
use crate::signal::{Signal, SignalOutcome};
use indexmap::IndexMap;

/// A named list of [`Signal`]s, built by [`crate::parse::Dialect::parse_audit`] from a spec's
/// `signals`.
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
    pub fn run(&self, run_log: &dyn RunLog) -> AuditReport {
        let reader = run_log.reader();
        let writer = run_log.writer();

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

            writer.push_metadata(Metadata {
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
    use crate::signal::SignalEval;
    use crate::{Input, RunLog, RunLogReader, RunLogWriter};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Debug)]
    struct MockSignal(SignalOutcome);

    impl Signal for MockSignal {
        fn evaluate(&self, _run_log: &dyn RunLogReader) -> SignalOutcome {
            self.0.clone()
        }
    }

    /// A `RunLog` whose only job is to capture every `Metadata` row pushed to it, so a test can
    /// assert on exactly what `Audit::run` writes back - `SimpleEventRunLog` has no way to read
    /// its metadata back out, and `SqliteRunLog` needs a real directory on disk.
    #[derive(Debug, Default)]
    struct MockRunLog {
        metadata: Rc<RefCell<Vec<Metadata>>>,
    }

    #[derive(Debug)]
    struct MockRunLogHandle(Rc<RefCell<Vec<Metadata>>>);

    impl RunLog for MockRunLog {
        fn reader(&self) -> Rc<dyn RunLogReader> {
            Rc::new(MockRunLogHandle(Rc::clone(&self.metadata)))
        }

        fn writer(&self) -> Rc<dyn RunLogWriter> {
            Rc::new(MockRunLogHandle(Rc::clone(&self.metadata)))
        }
    }

    impl RunLogReader for MockRunLogHandle {
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

    impl RunLogWriter for MockRunLogHandle {
        fn push_input(&self, _input: Input) {
            unimplemented!("not exercised by these tests")
        }

        fn push_output(&self, _output: crate::Output) {
            unimplemented!("not exercised by these tests")
        }

        fn push_metadata(&self, metadata: Metadata) {
            self.0.borrow_mut().push(metadata);
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
        let audit = Audit::new(signals);

        let run_log = MockRunLog::default();
        let audit_report = audit.run(&run_log);

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
        let audit = Audit::new(signals);

        let run_log = MockRunLog::default();
        audit.run(&run_log);

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
