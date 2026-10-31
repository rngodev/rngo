use chrono::{DateTime, FixedOffset};

/// What a run is parameterized by besides time windows: the `seed` and the `now` that relative moments resolve against.
#[derive(Clone, Debug)]
pub struct Scenario {
    pub seed: u64,
    pub now: DateTime<FixedOffset>,
}
