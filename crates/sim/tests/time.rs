mod common;

use rngo_sim::build::*;
use rngo_sim::Simulation;
use serde_json::Value;

fn effect_offsets(sim: Simulation, take: usize) -> Vec<u64> {
    sim.take(take).map(|e| e.offset).collect()
}

/// The default simulation window is 30 days (start = -30d, end = now).
/// Offsets are cumulative seconds from the start, so all events should
/// have offset <= 30 * 86_400 = 2_592_000.
///
/// With the default clock rate of 1 event/day, taking 60 events drives
/// the simulation ~60 days into the future — well past the end.
#[test]
fn simulation_respects_end_time() {
    let mut builder = Simulation::builder();
    builder.with_effect("events", |e| {
        e.set_schema(constant().value(Value::Null));
    });

    let offsets = effect_offsets(builder.build().unwrap(), 60);
    let window_secs: u64 = 30 * 86_400;

    let out_of_bounds: Vec<_> = offsets
        .iter()
        .copied()
        .filter(|&o| o > window_secs)
        .collect();
    assert!(
        out_of_bounds.is_empty(),
        "{} events past end time (>{window_secs}s), e.g. {}",
        out_of_bounds.len(),
        out_of_bounds[0],
    );
}

/// An effect with its own start offset should not emit events before that offset,
/// even though the simulation window begins earlier.
#[test]
fn effect_respects_start_time() {
    use chrono::TimeDelta;
    use rngo_sim::Moment;

    let mut builder = Simulation::builder();
    // Simulation: -30d to now. Effect starts at -15d (halfway through).
    builder.with_effect("events", |e| {
        e.set_start(Moment::Relative(TimeDelta::days(-15)));
        e.set_schema(constant().value(Value::Null));
    });

    let offsets = effect_offsets(builder.build().unwrap(), 60);

    // Effect start is 15 days into the 30-day window = 15 * 86_400 seconds.
    let effect_start_offset: u64 = 15 * 86_400;

    let too_early: Vec<_> = offsets
        .iter()
        .copied()
        .filter(|&o| o < effect_start_offset)
        .collect();
    assert!(
        too_early.is_empty(),
        "{} events before effect start (<{effect_start_offset}s), e.g. {}",
        too_early.len(),
        too_early[0],
    );
}
