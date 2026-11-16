use rngo::{Dialect, SimpleEventRunLog};

/// The README's JSON spec parses into all three builders and the simulation builds, so it can't drift from the spec format.
#[test]
fn readme_spec_is_valid() {
    let readme = include_str!("../README.md");
    let raw = readme
        .split("```json\n")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("README has a json block");

    let spec = rngo::spec::from_value(serde_json::from_str(raw).unwrap()).unwrap();
    let dialect = Dialect::primitive();

    dialect
        .parse_simulation(spec.clone())
        .unwrap()
        .run_log_reader(SimpleEventRunLog::new())
        .build()
        .unwrap();
    dialect.parse_proxy(spec.clone()).unwrap();
    dialect.parse_audit(spec).unwrap();
}
