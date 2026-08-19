use rngo_sim::build::*;
use rngo_sim::{SimpleEventLog, Simulation, SqliteProxyLog};
use rusqlite::Connection;
use tempfile::TempDir;

#[test]
fn reference_with_no_prior_events_is_skipped_not_logged() {
    let tmp = TempDir::new().unwrap();
    let log = SqliteProxyLog::new(
        Box::new(SimpleEventLog::default()),
        tmp.path().to_path_buf(),
    );

    let mut simulation_builder = Simulation::builder();
    simulation_builder.with_effect("derived", |e| {
        e.trigger_hertz(1.0)
            .schema(reference().effect("nonexistent"))
    });

    let simulation = simulation_builder.log(log).build().unwrap();
    let events: Vec<_> = simulation.take(5).collect();

    assert!(
        events.is_empty(),
        "reference to an effect with no events should never yield a value"
    );

    let conn = Connection::open(tmp.path().join("log.sqlite")).unwrap();
    let effect_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM effects", [], |row| row.get(0))
        .unwrap();
    let error_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM errors", [], |row| row.get(0))
        .unwrap();

    assert_eq!(
        effect_count, 0,
        "skipped occurrences must not be logged as effects"
    );
    assert_eq!(
        error_count, 0,
        "skipped occurrences must not be logged as errors"
    );
}

#[test]
fn object_with_a_skipped_property_is_itself_skipped() {
    let mut simulation_builder = Simulation::builder();
    simulation_builder.with_effect("derived", |e| {
        e.trigger_hertz(1.0).schema(
            object()
                .property("id", constant().value(1))
                .property("missing", reference().effect("nonexistent")),
        )
    });

    let simulation = simulation_builder.build().unwrap();
    let events: Vec<_> = simulation.take(5).collect();

    assert!(
        events.is_empty(),
        "an object with any skipped property should itself be skipped, not emitted partially"
    );
}

#[test]
fn array_with_a_skipped_item_is_itself_skipped() {
    let mut simulation_builder = Simulation::builder();
    simulation_builder.with_effect("derived", |e| {
        e.trigger_hertz(1.0).schema(
            array()
                .min_items(1)
                .max_items(1)
                .items(reference().effect("nonexistent")),
        )
    });

    let simulation = simulation_builder.build().unwrap();
    let events: Vec<_> = simulation.take(5).collect();

    assert!(
        events.is_empty(),
        "an array with any skipped item should itself be skipped, not emitted partially"
    );
}
