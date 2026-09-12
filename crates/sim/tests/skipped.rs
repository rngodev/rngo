use rngo_sim::build::*;
use rngo_sim::{SimpleEventRunLog, Simulation, SqliteRunLog};
use rusqlite::Connection;
use tempfile::TempDir;

#[test]
fn reference_with_no_prior_events_is_skipped_not_logged() {
    let tmp = TempDir::new().unwrap();
    let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);

    let mut simulation_builder = Simulation::builder();
    simulation_builder.with_effect("derived", |e| {
        e.trigger_hertz(1.0)
            .schema(reference().effect("nonexistent"))
    });

    let simulation = simulation_builder
        .run_log(&run_log)
        .limit(5)
        .build()
        .unwrap();

    // `Simulation` now writes every real input, and every skipped occurrence's metadata, to the
    // run log itself as it iterates (see `Simulation::next`).
    let input_count = simulation.count();

    assert_eq!(
        input_count, 0,
        "reference to an effect with no events should never yield a value"
    );

    // Dropping the run log commits its pending transaction (see `SqliteRunLog`'s `Drop` impl),
    // so its writes are visible to a fresh connection opened on the same file.
    drop(run_log);

    let conn = Connection::open(tmp.path().join("log.sqlite")).unwrap();
    let input_row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM inputs", [], |row| row.get(0))
        .unwrap();
    let metadata_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM metadata", [], |row| row.get(0))
        .unwrap();

    assert_eq!(
        input_row_count, 0,
        "skipped occurrences must not be logged as inputs"
    );
    assert!(
        metadata_count > 0,
        "a skipped occurrence should still be logged as metadata"
    );
}

#[test]
fn object_with_a_skipped_property_is_itself_skipped() {
    let run_log = SimpleEventRunLog::new(1);

    let mut simulation_builder = Simulation::builder();
    simulation_builder.with_effect("derived", |e| {
        e.trigger_hertz(1.0).schema(
            object()
                .property("id", constant().value(1))
                .property("missing", reference().effect("nonexistent")),
        )
    });

    // `.limit(5)` bounds total attempts, not real inputs - without it, an effect that always
    // skips would loop internally until the simulation's time window itself runs out (see
    // `Simulation::next`), rather than stopping quickly.
    let simulation = simulation_builder
        .run_log(&run_log)
        .limit(5)
        .build()
        .unwrap();

    let events: Vec<_> = simulation.collect();

    assert!(
        events.is_empty(),
        "an object with any skipped property should itself be skipped, not emitted partially"
    );
}

#[test]
fn array_with_a_skipped_item_is_itself_skipped() {
    let run_log = SimpleEventRunLog::new(1);

    let mut simulation_builder = Simulation::builder();
    simulation_builder.with_effect("derived", |e| {
        e.trigger_hertz(1.0).schema(
            array()
                .min_items(1)
                .max_items(1)
                .items(reference().effect("nonexistent")),
        )
    });

    let simulation = simulation_builder
        .run_log(&run_log)
        .limit(5)
        .build()
        .unwrap();

    let events: Vec<_> = simulation.collect();

    assert!(
        events.is_empty(),
        "an array with any skipped item should itself be skipped, not emitted partially"
    );
}
