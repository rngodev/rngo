#![cfg(feature = "schema")]

use rngo::spec::{SCHEMA_NAMES, json_schema};
use std::path::PathBuf;

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/v1")
}

#[test]
fn committed_schemas_are_up_to_date() {
    let update = std::env::var_os("UPDATE_SCHEMA").is_some();
    for name in SCHEMA_NAMES {
        let path = schema_dir().join(format!("{name}.schema.json"));
        let expected = serde_json::to_string_pretty(&json_schema(name).unwrap()).unwrap() + "\n";
        if update {
            std::fs::write(&path, &expected).unwrap();
        } else {
            let actual = std::fs::read_to_string(&path).unwrap_or_default();
            assert_eq!(
                actual,
                expected,
                "{} is out of date; run `just schema`",
                path.display()
            );
        }
    }
}
