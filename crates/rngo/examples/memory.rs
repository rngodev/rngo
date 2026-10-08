use rngo::{Dialect, RunLogWriter, SqliteRunLog};
use serde_json::json;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

/// Global allocator that tracks current and peak heap usage.
struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(size: usize) {
    let current = CURRENT.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(current, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            grew(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size > layout.size() {
                grew(new_size - layout.size());
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn spec() -> serde_json::Value {
    json!({
        "seed": 1,
        "start": "2000-01-01",
        "end": "2100-01-01",
        "effects": {
            "user": {
                "trigger": "hz(1, minute)",
                "schema": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
            },
            "post": {
                "trigger": "hz(1, minute)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "author": { "type": "reference", "effect": "user", "cursor": "unique" },
                        "reviewer": { "type": "reference", "effect": "user" }
                    }
                }
            },
            "event": {
                "trigger": "hz(8, minute)",
                "schema": { "type": "number", "minimum": 0, "maximum": 100, "scale": 0 }
            }
        }
    })
}

/// Runs the spec on a `SqliteRunLog` for `limit` effect attempts, returning the number of inputs
/// produced and the peak heap bytes allocated during the run (excluding setup).
fn measure(limit: u64) -> (usize, usize) {
    let tmp = TempDir::new().unwrap();
    let log = SqliteRunLog::new(tmp.path().to_path_buf());
    let simulation = Dialect::primitive()
        .parse_simulation_json(spec())
        .unwrap()
        .limit(limit)
        .run_log_reader(log.clone())
        .build()
        .unwrap();

    let baseline = CURRENT.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);

    let mut inputs = 0;
    for input in simulation.flatten() {
        log.push_input(input);
        inputs += 1;
    }
    log.commit();

    (inputs, PEAK.load(Ordering::Relaxed) - baseline)
}

fn main() {
    let limits: Vec<u64> = std::env::args()
        .skip(1)
        .map(|arg| {
            arg.replace('_', "")
                .parse()
                .expect("limit must be a number")
        })
        .collect();
    let limits = if limits.is_empty() {
        vec![100_000, 1_000_000]
    } else {
        limits
    };

    println!(
        "{:>12} {:>12} {:>14} {:>12}",
        "limit", "inputs", "peak heap", "per input"
    );
    for limit in limits {
        let (inputs, peak) = measure(limit);
        println!(
            "{:>12} {:>12} {:>11.1} MB {:>10.1} B",
            limit,
            inputs,
            peak as f64 / 1_000_000.0,
            peak as f64 / inputs as f64
        );
    }
}
