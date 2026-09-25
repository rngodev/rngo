use crate::Output;
use crate::effect::Input;
use crate::output::Level;
use crate::run_log::{Metadata, RunLogReader, RunLogWriter};
use chrono::{DateTime, Utc};
use rand::RngExt;
use rand_pcg::Pcg32;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

const BATCH_SIZE: usize = 500;

#[derive(Debug)]
pub struct SqliteRunLog {
    connection: RefCell<Connection>,
    pending: Cell<usize>,
}

impl SqliteRunLog {
    pub fn new(directory: PathBuf) -> Rc<Self> {
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
                    data TEXT NOT NULL,
                    metadata TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS outputs (
                    channel TEXT NOT NULL,
                    input_id INTEGER,
                    timestamp TEXT NOT NULL,
                    level TEXT NOT NULL,
                    data TEXT NOT NULL,
                    metadata TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS metadata (
                    type TEXT NOT NULL,
                    segment TEXT,
                    input_id INTEGER,
                    output_id INTEGER,
                    offset INTEGER,
                    data TEXT
                );

                CREATE INDEX IF NOT EXISTS idx_metadata_input_id ON metadata(input_id);
                CREATE INDEX IF NOT EXISTS idx_metadata_output_id ON metadata(output_id);

                CREATE INDEX IF NOT EXISTS idx_metadata_unique_reference
                    ON metadata(segment, input_id) WHERE type = '_unique_reference';

                BEGIN;
                ",
            )
            .unwrap();

        Rc::new(SqliteRunLog {
            connection: RefCell::new(connection),
            pending: Cell::new(0),
        })
    }

    pub fn commit(&self) {
        commit(&self.connection, &self.pending);
    }

    fn record(&self) {
        self.pending.set(self.pending.get() + 1);
        if self.pending.get() >= BATCH_SIZE {
            commit(&self.connection, &self.pending);
        }
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

fn insert_metadata_row(
    connection: &Connection,
    mtype: &str,
    input_id: Option<i64>,
    output_id: Option<i64>,
    offset: Option<u64>,
    data: Option<&serde_json::Value>,
    segment: Option<&str>,
) {
    connection
        .prepare_cached(
            "INSERT INTO metadata (type, input_id, output_id, offset, data, segment) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .unwrap()
        .execute(rusqlite::params![
            mtype,
            input_id,
            output_id,
            offset.map(|o| o as i64),
            data.map(|v| v.to_string()),
            segment,
        ])
        .unwrap();
}

fn insert_metadata(connection: &Connection, metadata: &Metadata) {
    insert_metadata_row(
        connection,
        &metadata.mtype,
        metadata.input_id,
        metadata.output_id,
        metadata.offset,
        metadata.data.as_ref(),
        metadata.segment.as_deref(),
    );
}

impl RunLogWriter for SqliteRunLog {
    fn push_input(&self, input: Input) {
        self.connection
            .borrow()
            .prepare_cached(
                "INSERT INTO inputs (id, effect, offset, data, metadata) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .unwrap()
            .execute(rusqlite::params![
                input.id as i64,
                input.effect,
                input.offset as i64,
                serde_json::to_string(&input.data).unwrap(),
                serde_json::to_string(&input.metadata).unwrap(),
            ])
            .unwrap();

        self.record();
    }

    fn push_output(&self, output: Output) {
        self.connection
            .borrow()
            .prepare_cached(
                "INSERT INTO outputs (input_id, timestamp, channel, level, data, metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
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
                serde_json::to_string(&output.metadata).unwrap(),
            ])
            .unwrap();
        self.record();
    }

    fn push_metadata(&self, metadata: Metadata) {
        insert_metadata(&self.connection.borrow(), &metadata);
        self.record();
    }
}

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

fn query_last(connection: &Connection) -> Option<Rc<Input>> {
    let row = connection
        .prepare_cached(
            "SELECT id, effect, offset, data, metadata FROM inputs ORDER BY id DESC LIMIT 1",
        )
        .unwrap()
        .query_row([], |row| {
            let id: i64 = row.get(0)?;
            let effect: String = row.get(1)?;
            let offset: i64 = row.get(2)?;
            let data: String = row.get(3)?;
            let metadata: String = row.get(4)?;
            Ok((id, effect, offset, data, metadata))
        })
        .optional()
        .unwrap()?;

    let (id, effect, offset, data, metadata) = row;

    Some(Rc::new(Input {
        id: id as u64,
        effect,
        offset: offset as u64,
        timestamp: placeholder_timestamp(),
        data: serde_json::from_str(&data).unwrap(),
        metadata: serde_json::from_str(&metadata).unwrap(),
    }))
}

fn query_last_for_effect(connection: &Connection, key: &str) -> Option<Rc<Input>> {
    let row = connection
        .prepare_cached(
            "SELECT id, offset, data, metadata FROM inputs WHERE effect = ?1 ORDER BY id DESC LIMIT 1",
        )
        .unwrap()
        .query_row(rusqlite::params![key], |row| {
            let id: i64 = row.get(0)?;
            let offset: i64 = row.get(1)?;
            let data: String = row.get(2)?;
            let metadata: String = row.get(3)?;
            Ok((id, offset, data, metadata))
        })
        .optional()
        .unwrap()?;

    let (id, offset, data, metadata) = row;

    Some(Rc::new(Input {
        id: id as u64,
        effect: key.to_string(),
        offset: offset as u64,
        timestamp: placeholder_timestamp(),
        data: serde_json::from_str(&data).unwrap(),
        metadata: serde_json::from_str(&metadata).unwrap(),
    }))
}

fn query_random_for_effect(
    connection: &Connection,
    key: &str,
    rng: &mut Pcg32,
) -> Option<Rc<Input>> {
    let count: i64 = connection
        .prepare_cached("SELECT COUNT(*) FROM inputs WHERE effect = ?1")
        .unwrap()
        .query_row(rusqlite::params![key], |row| row.get(0))
        .unwrap();

    if count == 0 {
        return None;
    }

    let offset_index = rng.random_range(0..count);
    let row = connection
        .prepare_cached(
            "SELECT id, offset, data, metadata FROM inputs WHERE effect = ?1 ORDER BY id ASC LIMIT 1 OFFSET ?2",
        )
        .unwrap()
        .query_row(rusqlite::params![key, offset_index], |row| {
            let id: i64 = row.get(0)?;
            let offset: i64 = row.get(1)?;
            let data: String = row.get(2)?;
            let metadata: String = row.get(3)?;
            Ok((id, offset, data, metadata))
        })
        .optional()
        .unwrap()?;

    let (id, offset, data, metadata) = row;

    Some(Rc::new(Input {
        id: id as u64,
        effect: key.to_string(),
        offset: offset as u64,
        timestamp: placeholder_timestamp(),
        data: serde_json::from_str(&data).unwrap(),
        metadata: serde_json::from_str(&metadata).unwrap(),
    }))
}

fn query_unique_for_effect(
    connection: &Connection,
    key: &str,
    cursor: &str,
    rng: &mut Pcg32,
) -> Option<Rc<Input>> {
    let count: i64 = connection
        .prepare_cached(
            "SELECT COUNT(*) FROM inputs i WHERE i.effect = ?1 AND NOT EXISTS (
                SELECT 1 FROM metadata m
                WHERE m.type = '_unique_reference' AND m.segment = ?2 AND m.input_id = i.id
            )",
        )
        .unwrap()
        .query_row(rusqlite::params![key, cursor], |row| row.get(0))
        .unwrap();

    if count == 0 {
        return None;
    }

    let offset_index = rng.random_range(0..count);
    let row = connection
        .prepare_cached(
            "SELECT i.id, i.offset, i.data, i.metadata FROM inputs i WHERE i.effect = ?1 AND NOT EXISTS (
                SELECT 1 FROM metadata m
                WHERE m.type = '_unique_reference' AND m.segment = ?2 AND m.input_id = i.id
            ) ORDER BY i.id ASC LIMIT 1 OFFSET ?3",
        )
        .unwrap()
        .query_row(rusqlite::params![key, cursor, offset_index], |row| {
            let id: i64 = row.get(0)?;
            let offset: i64 = row.get(1)?;
            let data: String = row.get(2)?;
            let metadata: String = row.get(3)?;
            Ok((id, offset, data, metadata))
        })
        .optional()
        .unwrap()?;

    let (id, offset, data, metadata) = row;

    insert_metadata_row(
        connection,
        "_unique_reference",
        Some(id),
        None,
        Some(offset as u64),
        None,
        Some(cursor),
    );

    Some(Rc::new(Input {
        id: id as u64,
        effect: key.to_string(),
        offset: offset as u64,
        timestamp: placeholder_timestamp(),
        data: serde_json::from_str(&data).unwrap(),
        metadata: serde_json::from_str(&metadata).unwrap(),
    }))
}

impl RunLogReader for SqliteRunLog {
    fn last(&self) -> Option<Rc<Input>> {
        query_last(&self.connection.borrow())
    }

    fn last_for_effect(&self, key: &str) -> Option<Rc<Input>> {
        query_last_for_effect(&self.connection.borrow(), key)
    }

    fn random_for_effect(&self, key: &str, rng: &mut Pcg32) -> Option<Rc<Input>> {
        query_random_for_effect(&self.connection.borrow(), key, rng)
    }

    fn unique_for_effect(&self, key: &str, cursor: &str, rng: &mut Pcg32) -> Option<Rc<Input>> {
        query_unique_for_effect(&self.connection.borrow(), key, cursor, rng)
    }

    fn query(&self, query: &str) -> Option<serde_json::Value> {
        self.connection
            .borrow()
            .query_row(query, [], |row| row.get::<_, rusqlite::types::Value>(0))
            .ok()
            .and_then(sql_value_to_json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::Input;
    use crate::effect::schema::Metadata as SchemaMetadata;
    use chrono::Utc;
    use tempfile::TempDir;

    fn open(directory: &std::path::Path) -> Connection {
        Connection::open(directory.join("log.sqlite")).unwrap()
    }

    #[test]
    fn writes_input_and_output_metadata_inline() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());

        run_log.push_input(Input {
            id: 1,
            effect: "ping".to_string(),
            offset: 42,
            timestamp: Utc::now().fixed_offset(),
            data: serde_json::json!({ "a": 1 }),
            metadata: vec![SchemaMetadata {
                mtype: "error".into(),
                attribute: None,
                data: Some(serde_json::json!({ "message": "partial value" })),
            }],
        });
        run_log.push_output(Output {
            input_id: Some(1),
            timestamp: Utc::now(),
            channel: "logger".to_string(),
            level: Level::Info,
            data: "hello".to_string(),
            metadata: vec![SchemaMetadata {
                mtype: "error".into(),
                attribute: None,
                data: Some(serde_json::json!({ "message": "delivery failed" })),
            }],
        });

        // Force the pending transaction closed so the rows are visible to a fresh connection.
        run_log.commit();

        let conn = open(tmp.path());

        let (effect, input_metadata): (String, String) = conn
            .query_row("SELECT effect, metadata FROM inputs", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(effect, "ping");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&input_metadata).unwrap(),
            serde_json::json!([{ "type": "error", "attribute": null, "data": { "message": "partial value" } }])
        );

        let (output_data, output_metadata): (String, String) = conn
            .query_row("SELECT data, metadata FROM outputs", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(output_data, "hello");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output_metadata).unwrap(),
            serde_json::json!([{ "type": "error", "attribute": null, "data": { "message": "delivery failed" } }])
        );

        // Neither push touches the standalone `metadata` table - that's only for metadata with no
        // input/output row of its own to be embedded in.
        let standalone_metadata_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM metadata", [], |row| row.get(0))
            .unwrap();
        assert_eq!(standalone_metadata_count, 0);
    }

    #[test]
    fn skipped_inputs_write_metadata_with_no_input_row() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());

        run_log.push_metadata(Metadata {
            mtype: "skipped".into(),
            input_id: None,
            output_id: None,
            offset: Some(42),
            data: None,
            segment: None,
        });

        run_log.commit();

        let conn = open(tmp.path());

        let input_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM inputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(input_count, 0);

        let (metadata_input_id, metadata_output_id, metadata_offset, metadata_type, metadata_data): (
            Option<i64>,
            Option<i64>,
            i64,
            String,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT input_id, output_id, offset, type, data FROM metadata",
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
        assert_eq!(metadata_output_id, None);
        assert_eq!(metadata_offset, 42);
        assert_eq!(metadata_type, "skipped");
        assert_eq!(metadata_data, None);
    }

    #[test]
    fn standalone_metadata_can_carry_an_output_id() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());

        run_log.push_metadata(Metadata {
            mtype: "delivery_failed".into(),
            input_id: None,
            output_id: Some(7),
            offset: None,
            data: None,
            segment: None,
        });

        run_log.commit();

        let conn = open(tmp.path());
        let output_id: Option<i64> = conn
            .query_row("SELECT output_id FROM metadata", [], |row| row.get(0))
            .unwrap();
        assert_eq!(output_id, Some(7));
    }

    #[test]
    fn batches_across_commits() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());

        for i in 0..(BATCH_SIZE * 2 + 3) {
            run_log.push_input(Input {
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
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());
        assert!(run_log.last().is_none());
    }

    #[test]
    fn last_returns_most_recently_inserted_input_including_uncommitted() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());

        run_log.push_input(Input {
            id: 1,
            effect: "ping".to_string(),
            offset: 0,
            timestamp: Utc::now().fixed_offset(),
            data: serde_json::json!(1),
            metadata: vec![],
        });
        // Not committed yet - `last` must still see it, since it queries the same connection and
        // effect.rs computes ids mid-batch.
        let last = run_log.last().unwrap();
        assert_eq!(last.id, 1);

        run_log.push_input(Input {
            id: 2,
            effect: "ping".to_string(),
            offset: 1,
            timestamp: Utc::now().fixed_offset(),
            data: serde_json::json!(2),
            metadata: vec![],
        });
        let last = run_log.last().unwrap();
        assert_eq!(last.id, 2);
        assert_eq!(last.data, serde_json::json!(2));
    }

    #[test]
    fn last_for_effect_only_returns_most_recent_matching_effect() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());

        for (i, effect) in [(1, "a"), (2, "b"), (3, "a")] {
            run_log.push_input(Input {
                id: i,
                effect: effect.to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            });
        }

        assert_eq!(run_log.last_for_effect("a").unwrap().id, 3);
        assert_eq!(run_log.last_for_effect("b").unwrap().id, 2);
        assert!(run_log.last_for_effect("c").is_none());
    }

    fn rng(seed: u64) -> Pcg32 {
        rand_seeder::Seeder::from(&format!("{seed}-run_log")).into_rng()
    }

    /// Builds a `SqliteRunLog`, populates it with ten inputs on effect "a", then draws
    /// `random_for_effect` `draws` times under an rng seeded from `seed`, returning the sampled
    /// ids.
    fn sampled_ids(seed: u64, draws: usize) -> Vec<u64> {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());
        let mut rng = rng(seed);

        for i in 1..=10u64 {
            run_log.push_input(Input {
                id: i,
                effect: "a".to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            });
        }

        (0..draws)
            .map(|_| run_log.random_for_effect("a", &mut rng).unwrap().id)
            .collect()
    }

    #[test]
    fn random_for_effect_is_deterministic_for_a_fixed_seed() {
        assert_eq!(sampled_ids(42, 5), sampled_ids(42, 5));
    }

    #[test]
    fn random_for_effect_differs_across_seeds() {
        assert_ne!(sampled_ids(1, 5), sampled_ids(2, 5));
    }

    #[test]
    fn random_for_effect_returns_none_when_no_matching_effect() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());
        let mut rng = rng(1);

        assert!(run_log.random_for_effect("nonexistent", &mut rng).is_none());
    }

    fn push_inputs(run_log: &SqliteRunLog, effect: &str, count: u64) {
        for i in 1..=count {
            run_log.push_input(Input {
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
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());
        push_inputs(&run_log, "a", 5);
        let mut rng = rng(1);

        let mut seen = std::collections::HashSet::new();
        for _ in 0..5 {
            let sampled = run_log.unique_for_effect("a", "cursor", &mut rng).unwrap();
            assert!(
                seen.insert(sampled.id),
                "id {} returned more than once",
                sampled.id
            );
        }

        assert!(run_log.unique_for_effect("a", "cursor", &mut rng).is_none());
    }

    #[test]
    fn unique_cursor_is_deterministic_for_a_fixed_seed() {
        fn draw_all(seed: u64) -> Vec<u64> {
            let tmp = TempDir::new().unwrap();
            let run_log = SqliteRunLog::new(tmp.path().to_path_buf());
            push_inputs(&run_log, "a", 10);
            let mut rng = rng(seed);

            std::iter::from_fn(|| {
                run_log
                    .unique_for_effect("a", "cursor", &mut rng)
                    .map(|e| e.id)
            })
            .collect()
        }

        assert_eq!(draw_all(42), draw_all(42));
    }

    #[test]
    fn unique_cursor_state_is_independent_per_cursor() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());
        push_inputs(&run_log, "a", 1);
        let mut rng = rng(1);

        assert_eq!(
            run_log
                .unique_for_effect("a", "cursor-a", &mut rng)
                .unwrap()
                .id,
            1
        );
        // A second, independent cursor over the same effect can still draw the same input.
        assert_eq!(
            run_log
                .unique_for_effect("a", "cursor-b", &mut rng)
                .unwrap()
                .id,
            1
        );
    }

    #[test]
    fn unique_cursor_consumed_markers_do_not_leak_into_input_metadata() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());
        push_inputs(&run_log, "a", 1);
        let mut rng = rng(1);

        run_log.unique_for_effect("a", "cursor", &mut rng).unwrap();

        let last = run_log.last().unwrap();
        assert!(last.metadata.is_empty());
    }

    #[test]
    fn get_signal_returns_query_value() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());
        push_inputs(&run_log, "a", 3);
        run_log.commit();

        let value = run_log.query("SELECT COUNT(*) FROM inputs");

        assert_eq!(value, Some(serde_json::json!(3)));
    }

    #[test]
    fn get_signal_returns_none_on_query_error() {
        let tmp = TempDir::new().unwrap();
        let run_log = SqliteRunLog::new(tmp.path().to_path_buf());

        let value = run_log.query("SELECT COUNT(*) FROM missing_table");

        assert_eq!(value, None);
    }
}
