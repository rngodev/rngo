use crate::effect::Input;
use crate::log::{LogIndex, LogIndexConfig, LogReader};
use crate::output::Level;
use crate::schema::Metadata;
use crate::util::json_pointer::JsonPointer;
use crate::{Log, LogEvent};
use chrono::{DateTime, Utc};
use rand::RngExt;
use rand_pcg::Pcg32;
use rand_seeder::Seeder;
use rusqlite::{Connection, OptionalExtension};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;

/// Number of pushed events to accumulate in a single transaction before committing.
const BATCH_SIZE: usize = 500;

/// The sole store of a run's inputs, outputs, and metadata, on disk at `<run_dir>/log.sqlite`.
/// The reader shares the writer's connection (rather than opening a second one) so that
/// mid-transaction lookups - e.g. `effect.rs` computing the next input id from `last()` before
/// the current batch commits - see the writer's pending, uncommitted rows. It also owns a single
/// RNG seeded from the simulation's seed, shared with every reader/index it hands out, so
/// `LogIndex::sample`'s random branch is reproducible for a given seed rather than drawing from
/// an unseeded global generator.
#[derive(Debug)]
pub struct SqliteLog {
    connection: Rc<RefCell<Connection>>,
    rng: Rc<RefCell<Pcg32>>,
    pending: usize,
}

impl SqliteLog {
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
                    effect TEXT,
                    offset INTEGER,
                    attribute TEXT,
                    data TEXT
                );

                BEGIN;
                ",
            )
            .unwrap();

        SqliteLog {
            connection: Rc::new(RefCell::new(connection)),
            rng: Rc::new(RefCell::new(
                Seeder::from(&format!("{seed}-log")).into_rng(),
            )),
            pending: 0,
        }
    }

    fn record(&mut self) {
        self.pending += 1;
        if self.pending >= BATCH_SIZE {
            self.commit();
        }
    }

    fn commit(&mut self) {
        if self.pending > 0 {
            self.connection
                .borrow()
                .execute_batch("COMMIT; BEGIN;")
                .unwrap();
            self.pending = 0;
        }
    }

    fn insert_metadata(
        &mut self,
        input_id: Option<i64>,
        effect: &str,
        offset: u64,
        metadata: &[Metadata],
    ) {
        let connection = self.connection.borrow();
        for m in metadata {
            connection
                .prepare_cached(
                    "INSERT INTO metadata (input_id, effect, offset, type, attribute, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .unwrap()
                .execute(rusqlite::params![
                    input_id,
                    effect,
                    offset as i64,
                    m.mtype,
                    m.attribute.as_ref().map(|a| a.to_string()),
                    m.data.as_ref().map(|v| v.to_string()),
                ])
                .unwrap();
        }
    }
}

impl Log for SqliteLog {
    fn push(&mut self, event: LogEvent) {
        match &event {
            LogEvent::Input(e) => {
                self.connection
                    .borrow()
                    .prepare_cached(
                        "INSERT INTO inputs (id, effect, offset, data) VALUES (?1, ?2, ?3, ?4)",
                    )
                    .unwrap()
                    .execute(rusqlite::params![
                        e.id as i64,
                        e.effect,
                        e.offset as i64,
                        serde_json::to_string(&e.data).unwrap(),
                    ])
                    .unwrap();

                self.insert_metadata(Some(e.id as i64), &e.effect, e.offset, &e.metadata);
            }
            LogEvent::Skipped(e) => {
                self.insert_metadata(None, &e.effect, e.offset, &e.metadata);
            }
            LogEvent::Output(s) => {
                self.connection
                    .borrow()
                    .prepare_cached(
                        "INSERT INTO outputs (input_id, timestamp, channel, level, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                    )
                    .unwrap()
                    .execute(rusqlite::params![
                        s.input_id.map(|id| id as i64),
                        s.timestamp.to_rfc3339(),
                        s.channel,
                        match s.level {
                            Level::Error => "error",
                            Level::Warning => "warning",
                            Level::Info => "info",
                        },
                        s.data,
                    ])
                    .unwrap();
            }
        }
        self.record();
    }

    fn reader(&self) -> Rc<dyn LogReader> {
        Rc::new(SqliteLogReader {
            connection: Rc::clone(&self.connection),
            rng: Rc::clone(&self.rng),
        })
    }
}

impl Drop for SqliteLog {
    fn drop(&mut self) {
        let _ = self.connection.borrow().execute_batch("COMMIT;");
    }
}

/// The `inputs` table has no `timestamp` column, so rows reconstructed into an [`Input`] carry a
/// placeholder epoch timestamp. This is safe because [`LogReader::last`] only reads `.id`
/// (`effect.rs`) and [`LogIndex::sample`] only reads `.data`/`.metadata`
/// (`schema/reference.rs`) - nothing downstream reads a reconstructed `Input`'s timestamp.
fn placeholder_timestamp() -> DateTime<chrono::FixedOffset> {
    DateTime::<Utc>::UNIX_EPOCH.fixed_offset()
}

fn metadata_for_input(connection: &Connection, input_id: i64) -> Vec<Metadata> {
    connection
        .prepare_cached("SELECT type, attribute, data FROM metadata WHERE input_id = ?1")
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

#[derive(Debug)]
struct SqliteLogReader {
    connection: Rc<RefCell<Connection>>,
    rng: Rc<RefCell<Pcg32>>,
}

impl LogReader for SqliteLogReader {
    fn last(&self) -> Option<Rc<Input>> {
        let connection = self.connection.borrow();
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
        let metadata = metadata_for_input(&connection, id);

        Some(Rc::new(Input {
            id: id as u64,
            effect,
            offset: offset as u64,
            timestamp: placeholder_timestamp(),
            data: serde_json::from_str(&data).unwrap(),
            metadata,
        }))
    }

    fn index(&self, config: LogIndexConfig) -> Box<dyn LogIndex> {
        Box::new(SqliteLogIndex {
            connection: Rc::clone(&self.connection),
            rng: Rc::clone(&self.rng),
            config,
        })
    }
}

#[derive(Debug)]
struct SqliteLogIndex {
    connection: Rc<RefCell<Connection>>,
    rng: Rc<RefCell<Pcg32>>,
    config: LogIndexConfig,
}

impl LogIndex for SqliteLogIndex {
    fn sample(&self) -> Option<Rc<Input>> {
        let connection = self.connection.borrow();

        let LogIndexConfig::ByEffect { key, last_only } = &self.config;

        let row = if *last_only {
            connection
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
                .unwrap()
        } else {
            let count: i64 = connection
                .prepare_cached("SELECT COUNT(*) FROM inputs WHERE effect = ?1")
                .unwrap()
                .query_row(rusqlite::params![key], |row| row.get(0))
                .unwrap();

            if count == 0 {
                None
            } else {
                let offset_index = self.rng.borrow_mut().random_range(0..count);
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
        }?;

        let (id, offset, data) = row;
        let metadata = metadata_for_input(&connection, id);

        Some(Rc::new(Input {
            id: id as u64,
            effect: key.clone(),
            offset: offset as u64,
            timestamp: placeholder_timestamp(),
            data: serde_json::from_str(&data).unwrap(),
            metadata,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{Input, SkippedInput};
    use chrono::Utc;
    use tempfile::TempDir;

    fn open(directory: &std::path::Path) -> Connection {
        Connection::open(directory.join("log.sqlite")).unwrap()
    }

    #[test]
    fn writes_input_output_and_metadata_rows() {
        let tmp = TempDir::new().unwrap();
        let mut log = SqliteLog::new(tmp.path().to_path_buf(), 1);

        log.push(LogEvent::Input(Input {
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
        }));
        log.push(LogEvent::Output(crate::output::Output {
            input_id: Some(1),
            timestamp: Utc::now(),
            channel: "logger".to_string(),
            level: Level::Info,
            data: "hello".to_string(),
        }));

        // Force the pending transaction closed so the rows are visible to a fresh connection.
        log.commit();

        let conn = open(tmp.path());

        let effect: String = conn
            .query_row("SELECT effect FROM inputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(effect, "ping");

        let output_data: String = conn
            .query_row("SELECT data FROM outputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(output_data, "hello");

        let (metadata_input_id, metadata_effect, metadata_offset, metadata_type, metadata_data): (
            i64,
            String,
            i64,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT input_id, effect, offset, type, data FROM metadata",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(metadata_input_id, 1);
        assert_eq!(metadata_effect, "ping");
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
        let mut log = SqliteLog::new(tmp.path().to_path_buf(), 1);

        log.push(LogEvent::Skipped(SkippedInput {
            effect: "ping".to_string(),
            offset: 42,
            timestamp: Utc::now().fixed_offset(),
            metadata: vec![Metadata {
                mtype: "skipped".into(),
                attribute: None,
                data: None,
            }],
        }));

        log.commit();

        let conn = open(tmp.path());

        let input_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM inputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(input_count, 0);

        let (metadata_input_id, metadata_effect, metadata_offset, metadata_type, metadata_data): (
            Option<i64>,
            String,
            i64,
            String,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT input_id, effect, offset, type, data FROM metadata",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(metadata_input_id, None);
        assert_eq!(metadata_effect, "ping");
        assert_eq!(metadata_offset, 42);
        assert_eq!(metadata_type, "skipped");
        assert_eq!(metadata_data, None);
    }

    #[test]
    fn batches_across_commits() {
        let tmp = TempDir::new().unwrap();
        let mut log = SqliteLog::new(tmp.path().to_path_buf(), 1);

        for i in 0..(BATCH_SIZE * 2 + 3) {
            log.push(LogEvent::Input(Input {
                id: (i + 1) as u64,
                effect: "ping".to_string(),
                offset: i as u64,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            }));
        }
        log.commit();

        let conn = open(tmp.path());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM inputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, (BATCH_SIZE * 2 + 3) as i64);
    }

    #[test]
    fn last_returns_none_when_empty() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::new(tmp.path().to_path_buf(), 1);
        let reader = log.reader();
        assert!(reader.last().is_none());
    }

    #[test]
    fn last_returns_most_recently_inserted_input_including_uncommitted() {
        let tmp = TempDir::new().unwrap();
        let mut log = SqliteLog::new(tmp.path().to_path_buf(), 1);
        let reader = log.reader();

        log.push(LogEvent::Input(Input {
            id: 1,
            effect: "ping".to_string(),
            offset: 0,
            timestamp: Utc::now().fixed_offset(),
            data: serde_json::json!(1),
            metadata: vec![],
        }));
        // Not committed yet - the reader must still see it, since it shares the writer's
        // connection and effect.rs computes ids mid-batch.
        let last = reader.last().unwrap();
        assert_eq!(last.id, 1);

        log.push(LogEvent::Input(Input {
            id: 2,
            effect: "ping".to_string(),
            offset: 1,
            timestamp: Utc::now().fixed_offset(),
            data: serde_json::json!(2),
            metadata: vec![],
        }));
        let last = reader.last().unwrap();
        assert_eq!(last.id, 2);
        assert_eq!(last.data, serde_json::json!(2));
    }

    #[test]
    fn index_last_only_returns_most_recent_matching_effect() {
        let tmp = TempDir::new().unwrap();
        let mut log = SqliteLog::new(tmp.path().to_path_buf(), 1);
        let reader = log.reader();

        for (i, effect) in [(1, "a"), (2, "b"), (3, "a")] {
            log.push(LogEvent::Input(Input {
                id: i,
                effect: effect.to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            }));
        }

        let index = reader.index(LogIndexConfig::ByEffect {
            key: "a".to_string(),
            last_only: true,
        });
        let sampled = index.sample().unwrap();
        assert_eq!(sampled.id, 3);
    }

    /// Builds a `SqliteLog` under the given seed, populates it with ten inputs on effect "a",
    /// then samples that effect's index `draws` times, returning the sampled ids.
    fn sampled_ids(seed: u64, draws: usize) -> Vec<u64> {
        let tmp = TempDir::new().unwrap();
        let mut log = SqliteLog::new(tmp.path().to_path_buf(), seed);
        let reader = log.reader();

        for i in 1..=10u64 {
            log.push(LogEvent::Input(Input {
                id: i,
                effect: "a".to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            }));
        }

        let index = reader.index(LogIndexConfig::ByEffect {
            key: "a".to_string(),
            last_only: false,
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
        let log = SqliteLog::new(tmp.path().to_path_buf(), 1);
        let reader = log.reader();

        let index = reader.index(LogIndexConfig::ByEffect {
            key: "nonexistent".to_string(),
            last_only: false,
        });
        assert!(index.sample().is_none());
    }
}
