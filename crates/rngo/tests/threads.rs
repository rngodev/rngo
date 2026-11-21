use rngo::{
    Audit, AuditBuilder, Dialect, Proxy, ProxyBuilder, RunLogReader, SimpleEventRunLog, Simulation,
    SimulationBuilder, SqliteRunLog, StandaloneSimulation, Step,
};
use serde_json::json;
use std::sync::Arc;

fn assert_send<T: Send>() {}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn runs_and_builders_are_send() {
    assert_send::<Simulation>();
    assert_send::<StandaloneSimulation>();
    assert_send::<Proxy>();
    assert_send::<Audit>();
    assert_send::<SimulationBuilder>();
    assert_send::<ProxyBuilder>();
    assert_send::<AuditBuilder>();
    assert_send_sync::<Dialect>();
    assert_send_sync::<SimpleEventRunLog>();
    assert_send_sync::<SqliteRunLog>();
}

#[test]
fn a_run_built_on_one_thread_runs_on_another() {
    let spec = rngo::spec::from_value(json!({
        "effects": {
            "ping": {
                "trigger": "hz(1, hour)",
                "limit": 3,
                "channel": "out",
                "generator": { "type": "constant", "value": "hi" }
            }
        },
        "channels": { "out": { "type": "stdout" } },
        "signals": {
            "count": { "type": "sql", "query": "SELECT COUNT(*) FROM inputs", "expect": "result == 3" }
        }
    }))
    .unwrap();
    let dialect = Dialect::primitive();
    let dir = tempfile::tempdir().unwrap();
    let run_log = Arc::new(SqliteRunLog::in_dir(dir.path()).unwrap());

    let simulation = dialect
        .parse_simulation(&spec)
        .unwrap()
        .run_log_reader(run_log.clone())
        .build()
        .unwrap();
    let proxy = dialect
        .parse_proxy(&spec)
        .unwrap()
        .effect_channels(simulation.effect_channels())
        .run_log_writer(run_log.clone())
        .build()
        .unwrap();
    let audit = dialect
        .parse_audit(&spec)
        .unwrap()
        .run_log(run_log.clone())
        .build()
        .unwrap();

    let report = std::thread::spawn(move || {
        let mut simulation = simulation;
        let mut proxy = proxy;
        for step in &mut simulation {
            if let Step::Input(input) = step.unwrap() {
                proxy.send(&input).unwrap();
            }
        }
        proxy.finish().unwrap();
        audit.run().unwrap()
    })
    .join()
    .unwrap();

    assert!(report.passed());
    let count = run_log.query("SELECT COUNT(*) FROM inputs").unwrap();
    assert_eq!(count, json!(3));
}
