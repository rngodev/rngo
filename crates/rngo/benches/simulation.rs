use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rngo::{Dialect, SimpleEventRunLog, Simulation, SimulationBuilder, SqliteRunLog};
use serde_json::{Value, json};
use tempfile::TempDir;

fn user_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 },
            "name": { "type": "string", "pattern": "[A-Z][a-z]{3,10} [A-Z][a-z]{3,12}" },
            "age": {
                "type": "select",
                "options": [
                    { "weight": 3, "schema": { "type": "number", "minimum": 18, "maximum": 65, "scale": 0 } },
                    { "weight": 1, "schema": { "type": "constant", "value": null } }
                ]
            },
            "created_at": { "type": "context", "path": ["sim", "offset"] }
        }
    })
}

fn spec(cursor: &str) -> Value {
    json!({
        "seed": 1,
        "start": "2024-01-01",
        "end": "2025-01-01",
        "effects": {
            "user": {
                "trigger": "hz(1, minute)",
                "schema": user_schema()
            },
            "post": {
                "trigger": "hz(3, minute)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 },
                        "user_id": {
                            "type": "function",
                            "expression": "user.id",
                            "variables": {
                                "user": { "type": "reference", "effect": "user", "cursor": cursor }
                            }
                        },
                        "title": { "type": "string", "pattern": "Post: .{10,20}" },
                        "tags": {
                            "type": "array",
                            "minItems": 0,
                            "maxItems": 5,
                            "items": {
                                "type": "select",
                                "options": [
                                    { "weight": 1, "schema": { "type": "constant", "value": "a" } },
                                    { "weight": 1, "schema": { "type": "constant", "value": "b" } }
                                ]
                            }
                        },
                        "created_at": { "type": "context", "path": ["sim", "offset"] }
                    }
                }
            }
        }
    })
}

fn builder(spec: &Value, limit: u64) -> SimulationBuilder {
    Dialect::primitive()
        .parse_simulation_json(spec.clone())
        .unwrap()
        .limit(limit)
}

fn bench_spec(c: &mut Criterion, name: &str, spec: Value, sizes: &[u64]) {
    let mut group = c.benchmark_group(format!("simulation/{name}"));
    group.sample_size(10);

    for &size in sizes {
        group.throughput(Throughput::Elements(size));

        group.bench_with_input(BenchmarkId::new("memory", size), &size, |b, &size| {
            b.iter_batched(
                || {
                    builder(&spec, size)
                        .run_log(SimpleEventRunLog::new())
                        .build()
                        .unwrap()
                },
                Simulation::count,
                BatchSize::PerIteration,
            );
        });

        group.bench_with_input(BenchmarkId::new("sqlite", size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let tmp = TempDir::new().unwrap();
                    let log = SqliteRunLog::new(tmp.path().to_path_buf());
                    let simulation = builder(&spec, size).run_log(log.clone()).build().unwrap();
                    (tmp, log, simulation)
                },
                |(tmp, log, mut simulation)| {
                    let count = simulation.by_ref().count();
                    simulation.finish();
                    log.commit();
                    (tmp, log, simulation, count)
                },
                BatchSize::PerIteration,
            );
        });
    }

    group.finish();
}

fn random_reference(c: &mut Criterion) {
    bench_spec(c, "random_reference", spec("random"), &[1_000, 10_000]);
}

fn unique_reference(c: &mut Criterion) {
    bench_spec(c, "unique_reference", spec("unique"), &[1_000, 5_000]);
}

criterion_group!(benches, random_reference, unique_reference);
criterion_main!(benches);
