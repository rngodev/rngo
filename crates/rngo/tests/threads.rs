use rngo::{
    Audit, AuditBuilder, Dialect, LogReader, Proxy, ProxyBuilder, SimpleLog, Simulation,
    SimulationBuilder, SqliteLog, StandaloneSimulation, Step,
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
    assert_send_sync::<SimpleLog>();
    assert_send_sync::<SqliteLog>();
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
    let log = Arc::new(SqliteLog::in_dir(dir.path()).unwrap());

    let simulation = dialect
        .parse_simulation(&spec)
        .unwrap()
        .log_reader(log.clone())
        .build()
        .unwrap();
    let proxy = dialect
        .parse_proxy(&spec)
        .unwrap()
        .log_writer(log.clone())
        .build()
        .unwrap();
    let audit = dialect
        .parse_audit(&spec)
        .unwrap()
        .log(log.clone())
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
    let count = log.query("SELECT COUNT(*) FROM inputs").unwrap();
    assert_eq!(count, json!(3));
}
