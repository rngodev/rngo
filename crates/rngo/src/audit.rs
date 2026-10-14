use crate::build::BuildError;
use crate::log::Metadata;
pub mod probe;

use crate::{RunLogReader, RunLogWriter};
use indexmap::IndexMap;
use probe::{Probe, ProbeBuilder, ProbeOutcome, Signal};
use std::rc::Rc;

#[derive(Debug)]
pub struct Audit {
    probes: IndexMap<String, Box<dyn Probe>>,
    run_log_reader: Rc<dyn RunLogReader>,
    run_log_writer: Rc<dyn RunLogWriter>,
}

impl Audit {
    pub(crate) fn new(
        probes: IndexMap<String, Box<dyn Probe>>,
        run_log_reader: Rc<dyn RunLogReader>,
        run_log_writer: Rc<dyn RunLogWriter>,
    ) -> Self {
        Audit {
            probes,
            run_log_reader,
            run_log_writer,
        }
    }

    pub fn builder() -> AuditBuilder {
        AuditBuilder::new()
    }

    pub fn run(&self) -> AuditReport {
        let outcomes: IndexMap<String, ProbeOutcome> = self
            .probes
            .iter()
            .map(|(key, probe)| (key.clone(), probe.evaluate(self.run_log_reader.as_ref())))
            .collect();

        for (key, outcome) in &outcomes {
            match outcome {
                ProbeOutcome::Signals(signals) => {
                    for signal in signals {
                        self.run_log_writer.push_signal(key, signal.clone());
                    }
                }
                ProbeOutcome::Error { error } => {
                    self.run_log_writer.push_metadata(Metadata {
                        mtype: "probe_error".to_string(),
                        input_id: None,
                        output_id: None,
                        data: Some(serde_json::json!({ "key": key, "error": error })),
                        segment: None,
                        timestamp: None,
                    });
                }
            }
        }

        AuditReport { outcomes }
    }
}

pub struct AuditBuilder {
    probe_builders: Vec<(String, Box<dyn ProbeBuilder>)>,
    run_log_reader: Option<Rc<dyn RunLogReader>>,
    run_log_writer: Option<Rc<dyn RunLogWriter>>,
}

impl AuditBuilder {
    pub fn new() -> Self {
        AuditBuilder {
            probe_builders: vec![],
            run_log_reader: None,
            run_log_writer: None,
        }
    }

    pub fn with_probe(
        mut self,
        key: impl Into<String>,
        builder: impl ProbeBuilder + 'static,
    ) -> Self {
        self.set_probe(key, builder);
        self
    }

    pub fn set_probe(
        &mut self,
        key: impl Into<String>,
        builder: impl ProbeBuilder + 'static,
    ) -> &mut Self {
        self.probe_builders.push((key.into(), Box::new(builder)));
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
        let mut probes = IndexMap::new();

        for (key, builder) in self.probe_builders {
            match builder.build(&key) {
                Ok(probe) => {
                    probes.insert(key, probe);
                }
                Err(mut e) => errors.append(&mut e),
            }
        }

        // Unlike `SimulationBuilder`, which writes the run it produces to whatever run log it's
        // given (or a throwaway default), an `Audit` only ever reads and annotates a run log some
        // other process already populated - defaulting here would silently evaluate every probe
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
        Ok(Audit::new(probes, run_log_reader, run_log_writer))
    }
}

impl Default for AuditBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct AuditReport {
    pub outcomes: IndexMap<String, ProbeOutcome>,
}

impl AuditReport {
    pub fn passed(&self) -> bool {
        self.fail_count() == 0 && self.error_count() == 0
    }

    fn signals(&self) -> impl Iterator<Item = &Signal> {
        self.outcomes.values().flat_map(|outcome| match outcome {
            ProbeOutcome::Signals(signals) => signals.as_slice(),
            ProbeOutcome::Error { .. } => &[],
        })
    }

    pub fn eval_count(&self) -> usize {
        self.signals()
            .filter(|signal| signal.eval.is_some())
            .count()
    }

    pub fn pass_count(&self) -> usize {
        self.signals()
            .filter(|signal| signal.eval.is_some() && signal.passed())
            .count()
    }

    pub fn fail_count(&self) -> usize {
        self.signals().filter(|signal| !signal.passed()).count()
    }

    pub fn error_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|outcome| matches!(outcome, ProbeOutcome::Error { .. }))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Input, RunLogReader, RunLogWriter};
    use probe::{Signal, SignalEval};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Debug)]
    struct MockProbe(ProbeOutcome);

    impl Probe for MockProbe {
        fn evaluate(&self, _run_log: &dyn RunLogReader) -> ProbeOutcome {
            self.0.clone()
        }
    }

    /// A run log whose only job is to capture every `Metadata` row pushed to it, so a test can
    /// assert on exactly what `Audit::run` writes back - `SimpleEventRunLog` has no way to read
    /// its metadata back out, and `SqliteRunLog` needs a real directory on disk.
    #[derive(Debug, Default)]
    struct MockRunLog {
        metadata: RefCell<Vec<Metadata>>,
        signals: RefCell<Vec<(String, Signal)>>,
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

        fn push_signal(&self, probe: &str, signal: Signal) {
            self.signals.borrow_mut().push((probe.to_string(), signal));
        }
    }

    fn success(passed: bool) -> ProbeOutcome {
        ProbeOutcome::Signals(vec![Signal {
            value: serde_json::json!(1),
            eval: Some(SignalEval {
                expectation: "result == 1".to_string(),
                passed,
            }),
        }])
    }

    fn success_without_expect() -> ProbeOutcome {
        ProbeOutcome::Signals(vec![Signal {
            value: serde_json::json!(1),
            eval: None,
        }])
    }

    fn no_signals() -> ProbeOutcome {
        ProbeOutcome::Signals(vec![])
    }

    fn error() -> ProbeOutcome {
        ProbeOutcome::Error {
            error: "boom".to_string(),
        }
    }

    fn report(outcomes: Vec<(&str, ProbeOutcome)>) -> AuditReport {
        AuditReport {
            outcomes: outcomes
                .into_iter()
                .map(|(key, outcome)| (key.to_string(), outcome))
                .collect(),
        }
    }

    #[test]
    fn run_evaluates_every_probe_and_preserves_key_order() {
        let mut probes: IndexMap<String, Box<dyn Probe>> = IndexMap::new();
        probes.insert("b".to_string(), Box::new(MockProbe(success(true))));
        probes.insert("a".to_string(), Box::new(MockProbe(success(false))));

        let run_log = Rc::new(MockRunLog::default());
        let audit = Audit::new(probes, run_log.clone(), run_log);

        let audit_report = audit.run();

        let keys: Vec<_> = audit_report.outcomes.keys().collect();
        assert_eq!(keys, vec!["b", "a"]);
    }

    #[test]
    fn run_writes_each_signal_under_its_probe_key() {
        let mut probes: IndexMap<String, Box<dyn Probe>> = IndexMap::new();
        probes.insert("has-events".to_string(), Box::new(MockProbe(success(true))));
        probes.insert("silent".to_string(), Box::new(MockProbe(no_signals())));

        let run_log = Rc::new(MockRunLog::default());
        let audit = Audit::new(probes, run_log.clone(), run_log.clone());

        audit.run();

        let written = run_log.signals.borrow();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].0, "has-events");
        assert_eq!(written[0].1.value, serde_json::json!(1));
        assert!(run_log.metadata.borrow().is_empty());
    }

    #[test]
    fn run_writes_probe_errors_as_metadata_with_the_probe_key() {
        let mut probes: IndexMap<String, Box<dyn Probe>> = IndexMap::new();
        probes.insert("errored".to_string(), Box::new(MockProbe(error())));

        let run_log = Rc::new(MockRunLog::default());
        let audit = Audit::new(probes, run_log.clone(), run_log.clone());

        audit.run();

        let written = run_log.metadata.borrow();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].mtype, "probe_error");
        assert_eq!(
            written[0].data.as_ref().unwrap()["key"],
            serde_json::json!("errored")
        );
        assert_eq!(
            written[0].data.as_ref().unwrap()["error"],
            serde_json::json!("boom")
        );
        assert!(run_log.signals.borrow().is_empty());
    }

    #[test]
    fn builder_wires_probe_builders_into_a_runnable_audit() {
        use crate::build::sql_probe;

        let run_log = Rc::new(MockRunLog::default());
        let audit = Audit::builder()
            .with_probe("check", sql_probe().query("SELECT 1").expect("result == 1"))
            .run_log(run_log)
            .build()
            .unwrap();

        let report = audit.run();

        assert_eq!(report.outcomes.keys().collect::<Vec<_>>(), vec!["check"]);
    }

    #[test]
    fn builder_without_a_run_log_is_an_error() {
        use crate::build::sql_probe;

        let result = Audit::builder()
            .with_probe("check", sql_probe().query("SELECT 1"))
            .build();

        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn builder_collects_errors_from_every_failing_probe_builder() {
        use crate::build::sql_probe;

        let run_log = Rc::new(MockRunLog::default());
        let result = Audit::builder()
            .with_probe("no-query", sql_probe())
            .with_probe("also-no-query", sql_probe())
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
    fn a_probe_that_produces_no_signals_passes() {
        let report = report(vec![("silent", no_signals())]);
        assert!(report.passed());
        assert_eq!(report.eval_count(), 0);
        assert_eq!(report.error_count(), 0);
    }

    #[test]
    fn passed_is_false_when_any_eval_fails() {
        let report = report(vec![("a", success(true)), ("b", success(false))]);
        assert!(!report.passed());
    }

    #[test]
    fn passed_is_false_when_any_probe_errors() {
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
