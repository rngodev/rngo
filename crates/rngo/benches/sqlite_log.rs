use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rand_pcg::Pcg32;
use rngo::{Input, RunLogReader, RunLogWriter, SqliteRunLog};
use std::hint::black_box;
use std::rc::Rc;
use tempfile::TempDir;

const SIZES: [u64; 2] = [1_000, 10_000];

fn input(id: u64, effect: &str) -> Input {
    Input {
        id,
        effect: effect.to_string(),
        offset: id,
        timestamp: chrono::Utc::now().fixed_offset(),
        data: serde_json::json!({
            "id": id,
            "name": "Some User Name",
            "email": "user@example.com",
            "tags": ["a", "b", "c"],
        }),
        metadata: vec![],
    }
}

fn filled(count: u64) -> (TempDir, Rc<SqliteRunLog>) {
    let tmp = TempDir::new().unwrap();
    let log = SqliteRunLog::new(tmp.path().to_path_buf());
    for id in 1..=count {
        log.push_input(input(id, if id % 2 == 0 { "a" } else { "b" }));
    }
    log.commit();
    (tmp, log)
}

fn rng() -> Pcg32 {
    rand_seeder::Seeder::from("bench").into_rng()
}

fn push_input(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite_log/push_input");
    for size in SIZES {
        group.throughput(Throughput::Elements(size));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let tmp = TempDir::new().unwrap();
                    let log = SqliteRunLog::new(tmp.path().to_path_buf());
                    let inputs: Vec<_> = (1..=size).map(|id| input(id, "a")).collect();
                    (tmp, log, inputs)
                },
                |(tmp, log, inputs)| {
                    for input in inputs {
                        log.push_input(input);
                    }
                    log.commit();
                    (tmp, log)
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

fn last_for_effect(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite_log/last_for_effect");
    for size in SIZES {
        let (_tmp, log) = filled(size);
        group.bench_function(BenchmarkId::from_parameter(size), |b| {
            b.iter(|| black_box(log.last_for_effect("a")));
        });
    }
    group.finish();
}

fn random_for_effect(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite_log/random_for_effect");
    for size in SIZES {
        let (_tmp, log) = filled(size);
        let mut rng = rng();
        group.bench_function(BenchmarkId::from_parameter(size), |b| {
            b.iter(|| black_box(log.random_for_effect("a", &mut rng)));
        });
    }
    group.finish();
}

fn unique_for_effect(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite_log/unique_for_effect");
    for size in SIZES {
        let (_tmp, log) = filled(size);
        let mut rng = rng();
        let per_cursor = size / 4;
        let mut draws = 0u64;
        group.bench_function(BenchmarkId::from_parameter(size), |b| {
            b.iter(|| {
                let cursor = format!("cursor-{}", draws / per_cursor);
                draws += 1;
                black_box(log.unique_for_effect("a", &cursor, &mut rng))
            });
        });
    }
    group.finish();
}

fn query(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite_log/query");
    for size in SIZES {
        let (_tmp, log) = filled(size);
        group.bench_function(BenchmarkId::from_parameter(size), |b| {
            b.iter(|| {
                black_box(
                    log.query(
                        "SELECT COUNT(*) FROM inputs WHERE json_extract(data, '$.id') % 3 = 0",
                    ),
                )
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    push_input,
    last_for_effect,
    random_for_effect,
    unique_for_effect,
    query
);
criterion_main!(benches);
