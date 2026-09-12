use crate::effect::Input;
use crate::output::Level;
use crate::run_log::{Cursor, EffectMetadata, RunLogIndex, RunLogIndexConfig, RunLogReader};
use crate::schema::Metadata;
use crate::util::json_pointer::JsonPointer;
use crate::{Output, RunLog, RunLogWriter};
use chrono::{DateTime, Utc};
use rand::RngExt;
use rand_pcg::Pcg32;
use rand_seeder::Seeder;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;

/// Number of pushed events to accumulate in a single transaction before committing.
const BATCH_SIZE: usize = 500;

/// The sole store of a run's inputs, outputs, and metadata, on disk at `<run_dir>/log.sqlite`.
/// Every reader and writer it mints shares this same connection (rather than opening a new one
/// per handle) so that mid-transaction lookups - e.g. `effect.rs` computing the next input id
/// from `last()`, or a second writer persisting audit results - see every other handle's pending,
/// uncommitted rows. It also owns a single RNG seeded from the simulation's seed, shared with
/// every reader/index it hands out, so `RunLogIndex::sample`'s random branch is reproducible for
/// a given seed rather than drawing from an unseeded global generator.
#[derive(Debug)]
pub struct SqliteRunLog {
    connection: Rc<RefCell<Connection>>,
    rng: Rc<RefCell<Pcg32>>,
    /// Hands out a distinct id to each `Cursor::Unique` index, so their "already returned"
    /// bookkeeping in the `metadata` table doesn't collide.
    next_segment: Rc<Cell<u64>>,
    /// Shared with every writer handle, so any of them batching inserts trips the same commit
    /// threshold rather than each keeping an independent, under-counting total.
    pending: Rc<Cell<usize>>,
}

impl SqliteRunLog {
    pub fn new(directory: PathBuf, seed: u64) -> Self {
        let connection = Connection::open(directory.join("log.sqlite")).unwrap();

        connection
            .execute_batch(
                "
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;

                CREATE TABLE IF NOT EXISTS inputs (
                    id INTEGER NOT NULL,
                    effect TEXT NOT NULL,
                    offset INTEGER NOT NULL,
                    data TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS outputs (
                    channel TEXT NOT NULL,
                    input_id INTEGER,
                    timestamp TEXT NOT NULL,
                    level TEXT NOT NULL,
                    data TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS metadata (
                    type TEXT NOT NULL,
                    input_id INTEGER,
                    output_id INTEGER,
                    offset INTEGER,
                    attribute TEXT,
                    data TEXT,
                    segment TEXT
                );

                CREATE INDEX IF NOT EXISTS idx_metadata_input_id ON metadata(input_id);
                CREATE INDEX IF NOT EXISTS idx_metadata_output_id ON metadata(output_id);

                CREATE INDEX IF NOT EXISTS idx_metadata_unique_reference
                    ON metadata(segment, input_id) WHERE type = '_unique_reference';

                BEGIN;
                ",
            )
            .unwrap();

        SqliteRunLog {
            connection: Rc::new(RefCell::new(connection)),
            rng: Rc::new(RefCell::new(
                Seeder::from(&format!("{seed}-run_log")).into_rng(),
            )),
            next_segment: Rc::new(Cell::new(0)),
            pending: Rc::new(Cell::new(0)),
        }
    }

    /// Force-commits any writer's pending batch, so subsequent reads on a fresh connection (e.g.
    /// a test opening the file directly) see rows that haven't hit the `BATCH_SIZE` threshold.
    pub fn commit(&self) {
        commit(&self.connection, &self.pending);
    }
}

impl RunLog for SqliteRunLog {
    fn reader(&self) -> Rc<dyn RunLogReader> {
        Rc::new(SqliteRunLogReader {
            connection: Rc::clone(&self.connection),
            rng: Rc::clone(&self.rng),
            next_segment: Rc::clone(&self.next_segment),
        })
    }

    fn writer(&self) -> Rc<dyn RunLogWriter> {
        Rc::new(SqliteRunLogWriter {
            connection: Rc::clone(&self.connection),
            pending: Rc::clone(&self.pending),
        })
    }
}

impl Drop for SqliteRunLog {
    fn drop(&mut self) {
        let _ = self.connection.borrow().execute_batch("COMMIT;");
    }
}

fn commit(connection: &RefCell<Connection>, pending: &Cell<usize>) {
    if pending.get() > 0 {
        connection.borrow().execute_batch("COMMIT; BEGIN;").unwrap();
        pending.set(0);
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_metadata_row(
    connection: &Connection,
    mtype: &str,
    input_id: Option<i64>,
    output_id: Option<i64>,
    offset: Option<u64>,
    attribute: Option<&JsonPointer>,
    data: Option<&serde_json::Value>,
    segment: Option<&str>,
) {
    connection
        .prepare_cached(
            "INSERT INTO metadata (type, input_id, output_id, offset, attribute, data, segment) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .unwrap()
        .execute(rusqlite::params![
            mtype,
            input_id,
            output_id,
            offset.map(|o| o as i64),
            attribute.map(|a| a.to_string()),
            data.map(|v| v.to_string()),
            segment,
        ])
        .unwrap();
}

/// Inserts the single row an [`EffectMetadata`] describes (e.g. a skipped occurrence's entry,
/// logged with no `input_id`).
fn insert_effect_metadata(connection: &Connection, metadata: &EffectMetadata) {
    insert_metadata_row(
        connection,
        &metadata.mtype,
        metadata.input_id,
        metadata.output_id,
        metadata.offset,
        metadata.attribute.as_ref(),
        metadata.data.as_ref(),
        metadata.segment.as_deref(),
    );
}

#[derive(Debug)]
struct SqliteRunLogWriter {
    connection: Rc<RefCell<Connection>>,
    pending: Rc<Cell<usize>>,
}

impl SqliteRunLogWriter {
    fn record(&self) {
        self.pending.set(self.pending.get() + 1);
        if self.pending.get() >= BATCH_SIZE {
            commit(&self.connection, &self.pending);
        }
    }
}

impl RunLogWriter for SqliteRunLogWriter {
    fn push_input(&self, input: Input) {
        let connection = self.connection.borrow();
        connection
            .prepare_cached("INSERT INTO inputs (id, effect, offset, data) VALUES (?1, ?2, ?3, ?4)")
            .unwrap()
            .execute(rusqlite::params![
                input.id as i64,
                input.effect,
                input.offset as i64,
                serde_json::to_string(&input.data).unwrap(),
            ])
            .unwrap();

        for m in &input.metadata {
            insert_metadata_row(
                &connection,
                &m.mtype,
                Some(input.id as i64),
                None,
                Some(input.offset),
                m.attribute.as_ref(),
                m.data.as_ref(),
                None,
            );
        }

        drop(connection);
        self.record();
    }

    fn push_output(&self, output: Output) {
        self.connection
            .borrow()
            .prepare_cached(
                "INSERT INTO outputs (input_id, timestamp, channel, level, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .unwrap()
            .execute(rusqlite::params![
                output.input_id.map(|id| id as i64),
                output.timestamp.to_rfc3339(),
                output.channel,
                match output.level {
                    Level::Error => "error",
                    Level::Warning => "warning",
                    Level::Info => "info",
                },
                output.data,
            ])
            .unwrap();
        self.record();
    }

    fn push_metadata(&self, metadata: EffectMetadata) {
        insert_effect_metadata(&self.connection.borrow(), &metadata);
        self.record();
    }
}

/// The `inputs` table has no `timestamp` column, so rows reconstructed into an [`Input`] carry a
/// placeholder epoch timestamp. This is safe because [`RunLogReader::last`] only reads `.id`
/// (`effect.rs`) and [`RunLogIndex::sample`] only reads `.data`/`.metadata`
/// (`schema/reference.rs`) - nothing downstream reads a reconstructed `Input`'s timestamp.
fn placeholder_timestamp() -> DateTime<chrono::FixedOffset> {
    DateTime::<Utc>::UNIX_EPOCH.fixed_offset()
}

fn sql_value_to_json(value: SqlValue) -> Option<serde_json::Value> {
    Some(match value {
        SqlValue::Null => serde_json::Value::Null,
        SqlValue::Integer(i) => serde_json::Value::from(i),
        SqlValue::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        SqlValue::Text(s) => serde_json::Value::String(s),
        SqlValue::Blob(_) => return None,
    })
}

fn metadata_for_input(connection: &Connection, input_id: i64) -> Vec<Metadata> {
    connection
        .prepare_cached(
            "SELECT type, attribute, data FROM metadata WHERE input_id = ?1 AND type != '_unique_reference'",
        )
        .unwrap()
        .query_map(rusqlite::params![input_id], |row| {
            let mtype: String = row.get(0)?;
            let attribute: Option<String> = row.get(1)?;
            let data: Option<String> = row.get(2)?;
            Ok(Metadata {
                mtype,
                attribute: attribute.map(|a| JsonPointer::from_str(&a).unwrap()),
                data: data.map(|d| serde_json::from_str(&d).unwrap()),
            })
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

/// Backs [`RunLogReader::last`] - the most recently inserted input, visible to any writer's
/// pending, uncommitted rows since it's the shared connection.
fn query_last(connection: &Connection) -> Option<Rc<Input>> {
    let row = connection
        .prepare_cached("SELECT id, effect, offset, data FROM inputs ORDER BY id DESC LIMIT 1")
        .unwrap()
        .query_row([], |row| {
            let id: i64 = row.get(0)?;
            let effect: String = row.get(1)?;
            let offset: i64 = row.get(2)?;
            let data: String = row.get(3)?;
            Ok((id, effect, offset, data))
        })
        .optional()
        .unwrap()?;

    let (id, effect, offset, data) = row;
    let metadata = metadata_for_input(connection, id);

    Some(Rc::new(Input {
        id: id as u64,
        effect,
        offset: offset as u64,
        timestamp: placeholder_timestamp(),
        data: serde_json::from_str(&data).unwrap(),
        metadata,
    }))
}

/// Shared by [`RunLogIndex::sample`] and [`RunLog::sample_input`] - `segment` scopes
/// `Cursor::Unique`'s "already returned" bookkeeping and must already be reserved by the caller.
fn query_sample(
    connection: &Connection,
    rng: &RefCell<Pcg32>,
    segment: Option<&str>,
    config: &RunLogIndexConfig,
) -> Option<Rc<Input>> {
    let RunLogIndexConfig::ByEffect { key, cursor } = config;

    let row = match cursor {
        Cursor::Last => connection
            .prepare_cached(
                "SELECT id, offset, data FROM inputs WHERE effect = ?1 ORDER BY id DESC LIMIT 1",
            )
            .unwrap()
            .query_row(rusqlite::params![key], |row| {
                let id: i64 = row.get(0)?;
                let offset: i64 = row.get(1)?;
                let data: String = row.get(2)?;
                Ok((id, offset, data))
            })
            .optional()
            .unwrap(),
        Cursor::Random => {
            let count: i64 = connection
                .prepare_cached("SELECT COUNT(*) FROM inputs WHERE effect = ?1")
                .unwrap()
                .query_row(rusqlite::params![key], |row| row.get(0))
                .unwrap();

            if count == 0 {
                None
            } else {
                let offset_index = rng.borrow_mut().random_range(0..count);
                connection
                    .prepare_cached(
                        "SELECT id, offset, data FROM inputs WHERE effect = ?1 ORDER BY id ASC LIMIT 1 OFFSET ?2",
                    )
                    .unwrap()
                    .query_row(rusqlite::params![key, offset_index], |row| {
                        let id: i64 = row.get(0)?;
                        let offset: i64 = row.get(1)?;
                        let data: String = row.get(2)?;
                        Ok((id, offset, data))
                    })
                    .optional()
                    .unwrap()
            }
        }
        Cursor::Unique => {
            let segment = segment.expect("segment set for Cursor::Unique");

            let count: i64 = connection
                .prepare_cached(
                    "SELECT COUNT(*) FROM inputs i WHERE i.effect = ?1 AND NOT EXISTS (
                        SELECT 1 FROM metadata m
                        WHERE m.type = '_unique_reference' AND m.segment = ?2 AND m.input_id = i.id
                    )",
                )
                .unwrap()
                .query_row(rusqlite::params![key, segment], |row| row.get(0))
                .unwrap();

            if count == 0 {
                None
            } else {
                let offset_index = rng.borrow_mut().random_range(0..count);
                let row = connection
                    .prepare_cached(
                        "SELECT i.id, i.offset, i.data FROM inputs i WHERE i.effect = ?1 AND NOT EXISTS (
                            SELECT 1 FROM metadata m
                            WHERE m.type = '_unique_reference' AND m.segment = ?2 AND m.input_id = i.id
                        ) ORDER BY i.id ASC LIMIT 1 OFFSET ?3",
                    )
                    .unwrap()
                    .query_row(rusqlite::params![key, segment, offset_index], |row| {
                        let id: i64 = row.get(0)?;
                        let offset: i64 = row.get(1)?;
                        let data: String = row.get(2)?;
                        Ok((id, offset, data))
                    })
                    .optional()
                    .unwrap();

                if let Some((id, offset, _)) = &row {
                    insert_metadata_row(
                        connection,
                        "_unique_reference",
                        Some(*id),
                        None,
                        Some(*offset as u64),
                        None,
                        None,
                        Some(segment),
                    );
                }

                row
            }
        }
    }?;

    let (id, offset, data) = row;
    let metadata = metadata_for_input(connection, id);

    Some(Rc::new(Input {
        id: id as u64,
        effect: key.clone(),
        offset: offset as u64,
        timestamp: placeholder_timestamp(),
        data: serde_json::from_str(&data).unwrap(),
        metadata,
    }))
}

#[derive(Debug)]
struct SqliteRunLogReader {
    connection: Rc<RefCell<Connection>>,
    rng: Rc<RefCell<Pcg32>>,
    next_segment: Rc<Cell<u64>>,
}

impl RunLogReader for SqliteRunLogReader {
    fn last(&self) -> Option<Rc<Input>> {
        query_last(&self.connection.borrow())
    }

    /// Queries the shared connection, so pending, uncommitted events from this run - written by
    /// any writer sharing the store - are visible without needing a prior commit (see the struct
    /// docs). Only the raw query result is returned here - compiling/evaluating a signal's
    /// `expect` expression against it is backend-agnostic and lives in `signal/sql.rs`.
    fn query(&self, query: &str) -> Option<serde_json::Value> {
        self.connection
            .borrow()
            .query_row(query, [], |row| row.get::<_, rusqlite::types::Value>(0))
            .ok()
            .and_then(sql_value_to_json)
    }

    fn index(&self, config: RunLogIndexConfig) -> Box<dyn RunLogIndex> {
        let RunLogIndexConfig::ByEffect { cursor, .. } = &config;
        let segment = matches!(cursor, Cursor::Unique).then(|| {
            let segment = self.next_segment.get();
            self.next_segment.set(segment + 1);
            segment.to_string()
        });

        Box::new(SqliteRunLogIndex {
            connection: Rc::clone(&self.connection),
            rng: Rc::clone(&self.rng),
            segment,
            config,
        })
    }
}

#[derive(Debug)]
struct SqliteRunLogIndex {
    connection: Rc<RefCell<Connection>>,
    rng: Rc<RefCell<Pcg32>>,
    /// Assigned only under `Cursor::Unique`, to scope this index's "already returned" bookkeeping
    /// in the `metadata` table apart from any other `Cursor::Unique` index over the same effect.
    segment: Option<String>,
    config: RunLogIndexConfig,
}

impl RunLogIndex for SqliteRunLogIndex {
    fn sample(&self) -> Option<Rc<Input>> {
        query_sample(
            &self.connection.borrow(),
            &self.rng,
            self.segment.as_deref(),
            &self.config,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::Input;
    use chrono::Utc;
    use tempfile::TempDir;

    fn open(directory: &std::path::Path) -> Connection {
        Connection::open(directory.join("log.sqlite")).unwrap()
    }

    #[test]
    fn writes_input_output_and_metadata_rows() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);
        let writer = run_log.writer();

        writer.push_input(Input {
            id: 1,
            effect: "ping".to_string(),
            offset: 42,
            timestamp: Utc::now().fixed_offset(),
            data: serde_json::json!({ "a": 1 }),
            metadata: vec![Metadata {
                mtype: "error".into(),
                attribute: None,
                data: Some(serde_json::json!({ "message": "partial value" })),
            }],
        });
        writer.push_output(Output {
            input_id: Some(1),
            timestamp: Utc::now(),
            channel: "logger".to_string(),
            level: Level::Info,
            data: "hello".to_string(),
        });

        // Force the pending transaction closed so the rows are visible to a fresh connection.
        run_log.commit();

        let conn = open(tmp.path());

        let effect: String = conn
            .query_row("SELECT effect FROM inputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(effect, "ping");

        let output_data: String = conn
            .query_row("SELECT data FROM outputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(output_data, "hello");

        let (metadata_input_id, metadata_offset, metadata_type, metadata_data): (
            i64,
            i64,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT input_id, offset, type, data FROM metadata",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(metadata_input_id, 1);
        assert_eq!(metadata_offset, 42);
        assert_eq!(metadata_type, "error");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&metadata_data).unwrap(),
            serde_json::json!({ "message": "partial value" })
        );
    }

    #[test]
    fn skipped_inputs_write_metadata_with_no_input_row() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);
        let writer = run_log.writer();

        writer.push_metadata(EffectMetadata {
            mtype: "skipped".into(),
            input_id: None,
            output_id: None,
            offset: Some(42),
            attribute: None,
            data: None,
            segment: None,
        });

        run_log.commit();

        let conn = open(tmp.path());

        let input_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM inputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(input_count, 0);

        let (metadata_input_id, metadata_offset, metadata_type, metadata_data): (
            Option<i64>,
            i64,
            String,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT input_id, offset, type, data FROM metadata",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(metadata_input_id, None);
        assert_eq!(metadata_offset, 42);
        assert_eq!(metadata_type, "skipped");
        assert_eq!(metadata_data, None);
    }

    #[test]
    fn batches_across_commits() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);
        let writer = run_log.writer();

        for i in 0..(BATCH_SIZE * 2 + 3) {
            writer.push_input(Input {
                id: (i + 1) as u64,
                effect: "ping".to_string(),
                offset: i as u64,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            });
        }
        run_log.commit();

        let conn = open(tmp.path());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM inputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, (BATCH_SIZE * 2 + 3) as i64);
    }

    #[test]
    fn last_returns_none_when_empty() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);
        let reader = run_log.reader();
        assert!(reader.last().is_none());
    }

    #[test]
    fn last_returns_most_recently_inserted_input_including_uncommitted() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);
        let reader = run_log.reader();
        let writer = run_log.writer();

        writer.push_input(Input {
            id: 1,
            effect: "ping".to_string(),
            offset: 0,
            timestamp: Utc::now().fixed_offset(),
            data: serde_json::json!(1),
            metadata: vec![],
        });
        // Not committed yet - the reader must still see it, since it shares the writer's
        // connection and effect.rs computes ids mid-batch.
        let last = reader.last().unwrap();
        assert_eq!(last.id, 1);

        writer.push_input(Input {
            id: 2,
            effect: "ping".to_string(),
            offset: 1,
            timestamp: Utc::now().fixed_offset(),
            data: serde_json::json!(2),
            metadata: vec![],
        });
        let last = reader.last().unwrap();
        assert_eq!(last.id, 2);
        assert_eq!(last.data, serde_json::json!(2));
    }

    #[test]
    fn index_last_only_returns_most_recent_matching_effect() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);
        let reader = run_log.reader();
        let writer = run_log.writer();

        for (i, effect) in [(1, "a"), (2, "b"), (3, "a")] {
            writer.push_input(Input {
                id: i,
                effect: effect.to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            });
        }

        let index = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Last,
        });
        let sampled = index.sample().unwrap();
        assert_eq!(sampled.id, 3);
    }

    /// Builds a `SqliteRunLog` under the given seed, populates it with ten inputs on effect "a",
    /// then samples that effect's index `draws` times, returning the sampled ids.
    fn sampled_ids(seed: u64, draws: usize) -> Vec<u64> {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), seed);
        let reader = run_log.reader();
        let writer = run_log.writer();

        for i in 1..=10u64 {
            writer.push_input(Input {
                id: i,
                effect: "a".to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            });
        }

        let index = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Random,
        });

        (0..draws).map(|_| index.sample().unwrap().id).collect()
    }

    #[test]
    fn index_sample_is_deterministic_for_a_fixed_seed() {
        assert_eq!(sampled_ids(42, 5), sampled_ids(42, 5));
    }

    #[test]
    fn index_sample_differs_across_seeds() {
        assert_ne!(sampled_ids(1, 5), sampled_ids(2, 5));
    }

    #[test]
    fn index_sample_returns_none_when_no_matching_effect() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);
        let reader = run_log.reader();

        let index = reader.index(RunLogIndexConfig::ByEffect {
            key: "nonexistent".to_string(),
            cursor: Cursor::Random,
        });
        assert!(index.sample().is_none());
    }

    fn push_inputs(run_log: &SqliteRunLog, effect: &str, count: u64) {
        let writer = run_log.writer();
        for i in 1..=count {
            writer.push_input(Input {
                id: i,
                effect: effect.to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            });
        }
    }

    #[test]
    fn unique_cursor_never_repeats_and_exhausts() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);
        let reader = run_log.reader();
        push_inputs(&run_log, "a", 5);

        let index = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Unique,
        });

        let mut seen = std::collections::HashSet::new();
        for _ in 0..5 {
            let sampled = index.sample().unwrap();
            assert!(
                seen.insert(sampled.id),
                "id {} returned more than once",
                sampled.id
            );
        }

        assert!(index.sample().is_none());
    }

    #[test]
    fn unique_cursor_is_deterministic_for_a_fixed_seed() {
        fn draw_all(seed: u64) -> Vec<u64> {
            let tmp = TempDir::new().unwrap();
            let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), seed);
            let reader = run_log.reader();
            push_inputs(&run_log, "a", 10);

            let index = reader.index(RunLogIndexConfig::ByEffect {
                key: "a".to_string(),
                cursor: Cursor::Unique,
            });

            std::iter::from_fn(|| index.sample().map(|e| e.id)).collect()
        }

        assert_eq!(draw_all(42), draw_all(42));
    }

    #[test]
    fn unique_cursor_state_is_independent_per_index() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);
        let reader = run_log.reader();
        push_inputs(&run_log, "a", 1);

        let index_a = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Unique,
        });
        let index_b = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Unique,
        });

        assert_eq!(index_a.sample().unwrap().id, 1);
        // A second, independent unique index over the same effect can still draw the same input.
        assert_eq!(index_b.sample().unwrap().id, 1);
    }

    #[test]
    fn unique_cursor_consumed_markers_do_not_leak_into_input_metadata() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);
        let reader = run_log.reader();
        push_inputs(&run_log, "a", 1);

        let index = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Unique,
        });
        index.sample().unwrap();

        let last = reader.last().unwrap();
        assert!(last.metadata.is_empty());
    }

    #[test]
    fn get_signal_returns_query_value() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);
        push_inputs(&run_log, "a", 3);
        run_log.commit();

        let value = run_log.reader().query("SELECT COUNT(*) FROM inputs");

        assert_eq!(value, Some(serde_json::json!(3)));
    }

    #[test]
    fn get_signal_returns_none_on_query_error() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf(), 1);

        let value = run_log.reader().query("SELECT COUNT(*) FROM missing_table");

        assert_eq!(value, None);
    }
}
