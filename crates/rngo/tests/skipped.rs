use chrono::TimeDelta;
use rngo::build::*;
use rngo::{Input, Metadata, Moment, RunLogWriter, Simulation, SqliteRunLog};
use rusqlite::Connection;
use tempfile::TempDir;

fn short_window() -> Moment {
    Moment::Relative(TimeDelta::seconds(-5))
}

fn log_all(simulation: Simulation, log: &impl RunLogWriter) -> Vec<Input> {
    let mut inputs = vec![];
    for item in simulation {
        match item {
            Ok(input) => {
                log.push_input(input.clone());
                inputs.push(input);
            }
            Err(skipped) => log.push_metadata(Metadata::from(skipped)),
        }
    }
    inputs
}

#[test]
fn reference_with_no_prior_events_is_skipped_not_logged() {
    let tmp = TempDir::new().unwrap();
    let run_log = SqliteRunLog::new(tmp.path().to_path_buf());

    let mut simulation_builder = Simulation::builder();
    simulation_builder.with_effect("derived", |e| {
        e.trigger_hertz(1.0)
            .schema(reference().effect("nonexistent"))
    });

    let simulation = simulation_builder
        .run_log_reader(run_log.clone())
        .start(short_window())
        .build()
        .unwrap();

    let input_count = log_all(simulation, run_log.as_ref()).len();

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
    let mut simulation_builder = Simulation::builder();
    simulation_builder.with_effect("derived", |e| {
        e.trigger_hertz(1.0).schema(
            object()
                .property("id", constant().value(1))
                .property("missing", reference().effect("nonexistent")),
        )
    });

    // `.limit` only counts real inputs, so an effect that always skips runs until the simulation's
    // time window ends; keep the window short.
    let simulation = simulation_builder
        .start(short_window())
        .standalone()
        .unwrap();

    let events: Vec<_> = simulation.flatten().collect();

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

    let simulation = simulation_builder
        .start(short_window())
        .standalone()
        .unwrap();

    let events: Vec<_> = simulation.flatten().collect();

    assert!(
        events.is_empty(),
        "an array with any skipped item should itself be skipped, not emitted partially"
    );
}
