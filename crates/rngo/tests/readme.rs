use rngo::{Dialect, SimpleEventRunLog, SqliteRunLog, Step};
use serde_json::{Value, json};

fn readme_spec() -> Value {
    let readme = include_str!("../README.md").replace("\r\n", "\n");
    let raw = readme
        .split("```json\n")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("README has a json block");
    serde_json::from_str(raw).unwrap()
}

/// The README's JSON spec parses into all three builders and the simulation builds, so it can't drift from the spec format.
#[test]
fn readme_spec_is_valid() {
    let spec = rngo::spec::from_value(readme_spec()).unwrap();
    let dialect = Dialect::primitive();

    dialect
        .parse_simulation(&spec)
        .unwrap()
        .run_log_reader(SimpleEventRunLog::new())
        .build()
        .unwrap();
    dialect.parse_proxy(&spec).unwrap();
    dialect.parse_audit(&spec).unwrap();
}

/// The README's run loop and audit pass on its spec, with every channel swapped for `stdout` since the samples themselves are `no_run` (they spawn `psql` and `tail`).
#[test]
fn readme_spec_runs_and_passes_audit() {
    let mut value = readme_spec();
    for channel in value["channels"].as_object_mut().unwrap().values_mut() {
        let mut stdout = json!({ "type": "stdout" });
        if let Some(format) = channel.get("format") {
            stdout["format"] = format.clone();
        }
        *channel = stdout;
    }

    let spec = rngo::spec::from_value(value).unwrap();
    let dialect = Dialect::primitive();
    let dir = tempfile::tempdir().unwrap();
    let run_log = SqliteRunLog::new(dir.path().to_path_buf()).unwrap();

    let mut simulation = dialect
        .parse_simulation(&spec)
        .unwrap()
        .limit(200)
        .run_log_reader(run_log.clone())
        .build()
        .unwrap();

    let mut proxy = dialect
        .parse_proxy(&spec)
        .unwrap()
        .effect_channels(simulation.effect_channels())
        .run_log_writer(run_log.clone())
        .build()
        .unwrap();

    let audit = dialect
        .parse_audit(&spec)
        .unwrap()
        .run_log(run_log)
        .build()
        .unwrap();

    for step in &mut simulation {
        if let Step::Input(input) = step.unwrap() {
            proxy.send(&input).unwrap();
        }
    }

    proxy.finish().unwrap();

    let audit_report = audit.run().unwrap();
    assert!(audit_report.passed());
}
