use crate::effect::Input;
use crate::log::pool::InputPool;
use crate::log::{LogReader, LogWriter, Metadata};
use crate::{Level, LogError, Output, Rng};
use jiff::Timestamp;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

const BATCH_SIZE: usize = 500;

/// Converts `rusqlite` errors into [`LogError::Sqlite`], so `rusqlite::Error` needs no public
/// `From` impl.
trait SqliteResultExt<T> {
    fn sqlite(self) -> Result<T, LogError>;
}

impl<T> SqliteResultExt<T> for Result<T, rusqlite::Error> {
    fn sqlite(self) -> Result<T, LogError> {
        self.map_err(|error| LogError::Sqlite(Box::new(error)))
    }
}

/// Errors unless `connection` is a new database or a log of [`SqliteLog::VERSION`], before
/// anything is written to it.
fn check_version(connection: &Connection) -> Result<(), LogError> {
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .sqlite()?;
    let has_tables: bool = connection
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table')",
            [],
            |row| row.get(0),
        )
        .sqlite()?;

    match (version, has_tables) {
        (0, false) => Ok(()),
        (version, _) if version == SqliteLog::VERSION => Ok(()),
        (found, _) => Err(LogError::UnsupportedVersion {
            found: (found != 0).then_some(found),
            supported: SqliteLog::VERSION,
        }),
    }
}

/// A log persisted to `log.sqlite`, buffering writes in batched transactions. Its tables are a
/// public contract that SQL signals query, documented in
/// [`LOG.md`](https://github.com/rngodev/rngo/blob/main/LOG.md).
#[derive(Debug)]
pub struct SqliteLog {
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    connection: Connection,
    pending: usize,
    pool: InputPool<u64>,
    failure: Option<LogError>,
}

impl SqliteLog {
    /// The version of the table layout this rngo writes and reads, stored in each log's
    /// `PRAGMA user_version`.
    pub const VERSION: u32 = 1;

    /// Opens (creating if needed) `log.sqlite` in `directory`, which must already exist. An existing
    /// log with another layout version is left untouched and reported as
    /// [`LogError::UnsupportedVersion`].
    pub fn in_dir(directory: impl AsRef<Path>) -> Result<Self, LogError> {
        let connection = Connection::open(directory.as_ref().join("log.sqlite")).sqlite()?;
        check_version(&connection)?;

        connection
            .execute_batch(&format!(
                "
                PRAGMA user_version = {};
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;

                CREATE TABLE IF NOT EXISTS inputs (
                    id INTEGER NOT NULL,
                    effect TEXT NOT NULL,
                    timestamp INTEGER NOT NULL,
                    data TEXT NOT NULL,
                    metadata TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS outputs (
                    channel TEXT NOT NULL,
                    input_id INTEGER,
                    timestamp INTEGER NOT NULL,
                    level TEXT NOT NULL,
                    data TEXT NOT NULL,
                    metadata TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS metadata (
                    type TEXT NOT NULL,
                    segment TEXT,
                    timestamp INTEGER,
                    input_id INTEGER,
                    output_id INTEGER,
                    data TEXT
                );

                CREATE INDEX IF NOT EXISTS idx_inputs_id ON inputs(id);
                CREATE INDEX IF NOT EXISTS idx_inputs_effect_id ON inputs(effect, id);

                CREATE INDEX IF NOT EXISTS idx_metadata_input_id ON metadata(input_id);
                CREATE INDEX IF NOT EXISTS idx_metadata_output_id ON metadata(output_id);

                CREATE INDEX IF NOT EXISTS idx_metadata_unique_reference
                    ON metadata(segment, input_id) WHERE type = '_unique_reference';

                BEGIN;
                ",
                Self::VERSION
            ))
            .sqlite()?;

        Ok(SqliteLog {
            state: Mutex::new(State {
                connection,
                pending: 0,
                pool: InputPool::default(),
                failure: None,
            }),
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Commits pending writes, or returns the first failure a lookup has hit since the last check.
    pub fn commit(&self) -> Result<(), LogError> {
        let mut state = self.state();
        state.check()?;
        state.commit()
    }
}

impl State {
    fn check(&mut self) -> Result<(), LogError> {
        match self.failure.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn lookup<T>(&mut self, result: Result<Option<T>, LogError>) -> Option<T> {
        match result {
            Ok(found) => found,
            Err(error) => {
                self.failure.get_or_insert(error);
                None
            }
        }
    }

    fn track(&mut self, effect: &str) -> Result<(), LogError> {
        if !self.pool.is_tracked(effect) {
            load_effect(&self.connection, &mut self.pool, effect)?;
        }
        Ok(())
    }

    fn record(&mut self) -> Result<(), LogError> {
        self.pending += 1;
        if self.pending >= BATCH_SIZE {
            self.commit()?;
        }
        Ok(())
    }

    fn commit(&mut self) -> Result<(), LogError> {
        if self.pending > 0 {
            self.connection.execute_batch("COMMIT; BEGIN;").sqlite()?;
            self.pending = 0;
        }
        Ok(())
    }
}

impl Drop for SqliteLog {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        let _ = state.connection.execute_batch("COMMIT;");
    }
}

/// Starts tracking `effect` in `pool`, seeded with its logged input ids and replaying its
/// `_unique_reference` markers.
fn load_effect(
    connection: &Connection,
    pool: &mut InputPool<u64>,
    effect: &str,
) -> Result<(), LogError> {
    let ids = connection
        .prepare_cached("SELECT id FROM inputs WHERE effect = ?1 ORDER BY id ASC")
        .sqlite()?
        .query_map(rusqlite::params![effect], |row| row.get::<_, i64>(0))
        .sqlite()?
        .map(|id| id.map(|id| id as u64))
        .collect::<Result<_, _>>()
        .sqlite()?;
    pool.track(effect, ids);

    let mut markers = connection
        .prepare_cached(
            "SELECT DISTINCT m.segment, m.input_id FROM metadata m
        JOIN inputs i ON i.id = m.input_id
        WHERE m.type = '_unique_reference' AND i.effect = ?1",
        )
        .sqlite()?;
    let rows = markers
        .query_map(rusqlite::params![effect], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .sqlite()?;
    for row in rows {
        let (cursor, id) = row.sqlite()?;
        pool.mark(effect, &cursor, &(id as u64));
    }
    Ok(())
}

fn insert_metadata_row(
    connection: &Connection,
    kind: &str,
    input_id: Option<u64>,
    output_id: Option<u64>,
    data: Option<&serde_json::Value>,
    segment: Option<&str>,
    timestamp: Option<Timestamp>,
) -> Result<(), LogError> {
    connection
        .prepare_cached(
            "INSERT INTO metadata (type, input_id, output_id, data, segment, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        ).sqlite()?
        .execute(rusqlite::params![
            kind,
            input_id.map(|id| id as i64),
            output_id.map(|id| id as i64),
            data.map(|v| v.to_string()),
            segment,
            timestamp.map(|t| t.as_millisecond()),
        ]).sqlite()?;
    Ok(())
}

fn insert_metadata(connection: &Connection, metadata: &Metadata) -> Result<(), LogError> {
    insert_metadata_row(
        connection,
        &metadata.kind,
        metadata.input_id,
        metadata.output_id,
        metadata.data.as_ref(),
        metadata.segment.as_deref(),
        metadata.timestamp,
    )
}

impl LogWriter for SqliteLog {
    fn push_input(&self, input: Input) -> Result<(), LogError> {
        let mut state = self.state();
        state.check()?;
        state
            .connection
            .prepare_cached(
                "INSERT INTO inputs (id, effect, timestamp, data, metadata) VALUES (?1, ?2, ?3, ?4, ?5)",
            ).sqlite()?
            .execute(rusqlite::params![
                input.id as i64,
                input.effect,
                input.timestamp.as_millisecond(),
                serde_json::to_string(&input.data)?,
                serde_json::to_string(&input.metadata)?,
            ]).sqlite()?;

        state.pool.push(&input.effect, input.id);
        state.record()
    }

    fn push_output(&self, output: Output) -> Result<(), LogError> {
        let mut state = self.state();
        state.check()?;
        state
            .connection
            .prepare_cached(
                "INSERT INTO outputs (input_id, timestamp, channel, level, data, metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            ).sqlite()?
            .execute(rusqlite::params![
                output.input_id.map(|id| id as i64),
                output.timestamp.as_millisecond(),
                output.channel,
                match output.level {
                    Level::Error => "error",
                    Level::Warning => "warning",
                    Level::Info => "info",
                },
                output.data,
                serde_json::to_string(&output.metadata)?,
            ]).sqlite()?;
        state.record()
    }

    fn push_metadata(&self, metadata: Metadata) -> Result<(), LogError> {
        let mut state = self.state();
        state.check()?;
        insert_metadata(&state.connection, &metadata)?;
        state.record()
    }

    fn flush(&self) -> Result<(), LogError> {
        self.commit()
    }
}

fn timestamp_from_millis(millis: i64) -> Timestamp {
    Timestamp::from_millisecond(millis).unwrap_or(Timestamp::UNIX_EPOCH)
}

fn sql_value_to_json(value: SqlValue) -> Result<serde_json::Value, LogError> {
    Ok(match value {
        SqlValue::Null => serde_json::Value::Null,
        SqlValue::Integer(i) => serde_json::Value::from(i),
        SqlValue::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        SqlValue::Text(s) => serde_json::Value::String(s),
        SqlValue::Blob(_) => return Err(LogError::UnrepresentableValue),
    })
}

fn query_last(connection: &Connection) -> Result<Option<Arc<Input>>, LogError> {
    let row = connection
        .prepare_cached(
            "SELECT id, effect, timestamp, data, metadata FROM inputs ORDER BY id DESC LIMIT 1",
        )
        .sqlite()?
        .query_row([], |row| {
            let id: i64 = row.get(0)?;
            let effect: String = row.get(1)?;
            let timestamp: i64 = row.get(2)?;
            let data: String = row.get(3)?;
            let metadata: String = row.get(4)?;
            Ok((id, effect, timestamp, data, metadata))
        })
        .optional()
        .sqlite()?;

    let Some((id, effect, timestamp, data, metadata)) = row else {
        return Ok(None);
    };

    Ok(Some(Arc::new(Input {
        channel: None,
        id: id as u64,
        effect,
        timestamp: timestamp_from_millis(timestamp),
        data: serde_json::from_str(&data)?,
        metadata: serde_json::from_str(&metadata)?,
    })))
}

fn query_last_for_effect(
    connection: &Connection,
    key: &str,
) -> Result<Option<Arc<Input>>, LogError> {
    let row = connection
        .prepare_cached(
            "SELECT id, timestamp, data, metadata FROM inputs WHERE effect = ?1 ORDER BY id DESC LIMIT 1",
        ).sqlite()?
        .query_row(rusqlite::params![key], |row| {
            let id: i64 = row.get(0)?;
            let timestamp: i64 = row.get(1)?;
            let data: String = row.get(2)?;
            let metadata: String = row.get(3)?;
            Ok((id, timestamp, data, metadata))
        })
        .optional().sqlite()?;

    let Some((id, timestamp, data, metadata)) = row else {
        return Ok(None);
    };

    Ok(Some(Arc::new(Input {
        channel: None,
        id: id as u64,
        effect: key.to_string(),
        timestamp: timestamp_from_millis(timestamp),
        data: serde_json::from_str(&data)?,
        metadata: serde_json::from_str(&metadata)?,
    })))
}

fn query_by_id(connection: &Connection, key: &str, id: u64) -> Result<Input, LogError> {
    let (timestamp, data, metadata) = connection
        .prepare_cached("SELECT timestamp, data, metadata FROM inputs WHERE id = ?1")
        .sqlite()?
        .query_row(rusqlite::params![id as i64], |row| {
            let timestamp: i64 = row.get(0)?;
            let data: String = row.get(1)?;
            let metadata: String = row.get(2)?;
            Ok((timestamp, data, metadata))
        })
        .sqlite()?;

    Ok(Input {
        channel: None,
        id,
        effect: key.to_string(),
        timestamp: timestamp_from_millis(timestamp),
        data: serde_json::from_str(&data)?,
        metadata: serde_json::from_str(&metadata)?,
    })
}

fn query_random_for_effect(
    connection: &Connection,
    pool: &InputPool<u64>,
    key: &str,
    rng: &mut Rng,
) -> Result<Option<Arc<Input>>, LogError> {
    let Some(id) = pool.random(key, rng) else {
        return Ok(None);
    };
    Ok(Some(Arc::new(query_by_id(connection, key, id)?)))
}

fn query_unique_for_effect(
    connection: &Connection,
    pool: &mut InputPool<u64>,
    key: &str,
    cursor: &str,
    rng: &mut Rng,
) -> Result<Option<Arc<Input>>, LogError> {
    let Some(id) = pool.take(key, cursor, rng) else {
        return Ok(None);
    };
    let input = query_by_id(connection, key, id)?;

    insert_metadata_row(
        connection,
        "_unique_reference",
        Some(input.id),
        None,
        None,
        Some(cursor),
        None,
    )?;

    Ok(Some(Arc::new(input)))
}

impl LogReader for SqliteLog {
    fn last(&self) -> Option<Arc<Input>> {
        let mut state = self.state();
        let result = query_last(&state.connection);
        state.lookup(result)
    }

    fn last_for_effect(&self, key: &str) -> Option<Arc<Input>> {
        let mut state = self.state();
        let result = query_last_for_effect(&state.connection, key);
        state.lookup(result)
    }

    fn random_for_effect(&self, key: &str, rng: &mut Rng) -> Option<Arc<Input>> {
        let mut state = self.state();
        let result = state
            .track(key)
            .and_then(|()| query_random_for_effect(&state.connection, &state.pool, key, rng));
        state.lookup(result)
    }

    fn unique_for_effect(&self, key: &str, cursor: &str, rng: &mut Rng) -> Option<Arc<Input>> {
        let mut state = self.state();
        let state_ref = &mut *state;
        let result = state_ref.track(key).and_then(|()| {
            query_unique_for_effect(&state_ref.connection, &mut state_ref.pool, key, cursor, rng)
        });
        state.lookup(result)
    }

    fn query(&self, query: &str) -> Result<serde_json::Value, LogError> {
        let value = self
            .state()
            .connection
            .query_row(query, [], |row| row.get::<_, rusqlite::types::Value>(0))
            .sqlite()?;
        sql_value_to_json(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::Input;
    use crate::effect::generator::GeneratorMetadata;
    use jiff::Timestamp;
    use tempfile::TempDir;

    fn open(directory: &std::path::Path) -> Connection {
        Connection::open(directory.join("log.sqlite")).unwrap()
    }

    fn user_version(directory: &std::path::Path) -> u32 {
        open(directory)
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_new_log_records_its_version_and_reopens() {
        let tmp = TempDir::new().unwrap();
        SqliteLog::in_dir(tmp.path()).unwrap().commit().unwrap();
        assert_eq!(user_version(tmp.path()), SqliteLog::VERSION);

        SqliteLog::in_dir(tmp.path()).unwrap();
    }

    #[test]
    fn a_log_of_another_version_is_refused_and_left_unchanged() {
        let tmp = TempDir::new().unwrap();
        open(tmp.path())
            .execute_batch("CREATE TABLE inputs (id INTEGER); PRAGMA user_version = 2;")
            .unwrap();

        let error = SqliteLog::in_dir(tmp.path()).unwrap_err();

        assert!(matches!(
            error,
            LogError::UnsupportedVersion {
                found: Some(2),
                supported: 1,
                ..
            }
        ));
        assert!(error.to_string().contains("uses log version 2"), "{error}");
        assert_eq!(user_version(tmp.path()), 2);
        let journal_mode: String = open(tmp.path())
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "delete");
    }

    #[test]
    fn an_unversioned_log_is_refused() {
        let tmp = TempDir::new().unwrap();
        open(tmp.path())
            .execute_batch("CREATE TABLE inputs (id INTEGER);")
            .unwrap();

        let error = SqliteLog::in_dir(tmp.path()).unwrap_err();

        assert!(matches!(
            error,
            LogError::UnsupportedVersion { found: None, .. }
        ));
        assert!(
            error
                .to_string()
                .contains("written before logs were versioned"),
            "{error}"
        );
    }

    #[test]
    fn reads_back_input_timestamps_as_milliseconds() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();
        let timestamp = Timestamp::from_millisecond(1_700_000_000_123).unwrap();

        log.push_input(Input {
            channel: None,
            id: 1,
            effect: "ping".to_string(),
            timestamp,
            data: serde_json::json!(1),
            metadata: vec![],
        })
        .unwrap();

        assert_eq!(log.last().unwrap().timestamp, timestamp);
        assert_eq!(log.last_for_effect("ping").unwrap().timestamp, timestamp);
        let stored: i64 = log
            .query("SELECT timestamp FROM inputs")
            .unwrap()
            .as_i64()
            .unwrap();
        assert_eq!(stored, 1_700_000_000_123);
    }

    #[test]
    fn writes_input_and_output_metadata_inline() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();

        log.push_input(Input {
            channel: None,
            id: 1,
            effect: "ping".to_string(),
            timestamp: Timestamp::now(),
            data: serde_json::json!({ "a": 1 }),
            metadata: vec![GeneratorMetadata {
                kind: "error".into(),
                attribute: None,
                data: Some(serde_json::json!({ "message": "partial value" })),
            }],
        })
        .unwrap();
        let output_timestamp = Timestamp::from_millisecond(1_700_000_000_456).unwrap();
        log.push_output(Output {
            input_id: Some(1),
            timestamp: output_timestamp,
            channel: "logger".to_string(),
            level: Level::Info,
            data: "hello".to_string(),
            metadata: vec![GeneratorMetadata {
                kind: "error".into(),
                attribute: None,
                data: Some(serde_json::json!({ "message": "delivery failed" })),
            }],
        })
        .unwrap();

        // Force the pending transaction closed so the rows are visible to a fresh connection.
        log.commit().unwrap();

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
        let stored_output_timestamp: i64 = conn
            .query_row("SELECT timestamp FROM outputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stored_output_timestamp, 1_700_000_000_456);
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
        let log = SqliteLog::in_dir(tmp.path()).unwrap();

        log.push_metadata(Metadata {
            kind: "skipped".into(),
            input_id: None,
            output_id: None,
            data: None,
            segment: None,
            timestamp: Some(Timestamp::from_millisecond(1_700_000_000_123).unwrap()),
        })
        .unwrap();

        log.commit().unwrap();

        let conn = open(tmp.path());

        let input_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM inputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(input_count, 0);

        let (metadata_input_id, metadata_output_id, metadata_type, metadata_data): (
            Option<i64>,
            Option<i64>,
            String,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT input_id, output_id, type, data FROM metadata",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(metadata_input_id, None);
        assert_eq!(metadata_output_id, None);
        assert_eq!(metadata_type, "skipped");
        let metadata_timestamp: Option<i64> = conn
            .query_row("SELECT timestamp FROM metadata", [], |row| row.get(0))
            .unwrap();
        assert_eq!(metadata_timestamp, Some(1_700_000_000_123));
        assert_eq!(metadata_data, None);
    }

    #[test]
    fn standalone_metadata_can_carry_an_output_id() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();

        log.push_metadata(Metadata {
            kind: "delivery_failed".into(),
            input_id: None,
            output_id: Some(7),
            data: None,
            segment: None,
            timestamp: None,
        })
        .unwrap();

        log.commit().unwrap();

        let conn = open(tmp.path());
        let output_id: Option<i64> = conn
            .query_row("SELECT output_id FROM metadata", [], |row| row.get(0))
            .unwrap();
        assert_eq!(output_id, Some(7));
    }

    #[test]
    fn batches_across_commits() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();

        for i in 0..(BATCH_SIZE * 2 + 3) {
            log.push_input(Input {
                channel: None,
                id: (i + 1) as u64,
                effect: "ping".to_string(),
                timestamp: Timestamp::now(),
                data: serde_json::json!(i),
                metadata: vec![],
            })
            .unwrap();
        }
        log.commit().unwrap();

        let conn = open(tmp.path());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM inputs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, (BATCH_SIZE * 2 + 3) as i64);
    }

    #[test]
    fn last_returns_none_when_empty() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();
        assert!(log.last().is_none());
    }

    #[test]
    fn last_returns_most_recently_inserted_input_including_uncommitted() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();

        log.push_input(Input {
            channel: None,
            id: 1,
            effect: "ping".to_string(),
            timestamp: Timestamp::now(),
            data: serde_json::json!(1),
            metadata: vec![],
        })
        .unwrap();
        // Not committed yet - `last` must still see it, since it queries the same connection and
        // effect.rs computes ids mid-batch.
        let last = log.last().unwrap();
        assert_eq!(last.id, 1);

        log.push_input(Input {
            channel: None,
            id: 2,
            effect: "ping".to_string(),
            timestamp: Timestamp::now(),
            data: serde_json::json!(2),
            metadata: vec![],
        })
        .unwrap();
        let last = log.last().unwrap();
        assert_eq!(last.id, 2);
        assert_eq!(last.data, serde_json::json!(2));
    }

    #[test]
    fn last_for_effect_only_returns_most_recent_matching_effect() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();

        for (i, effect) in [(1, "a"), (2, "b"), (3, "a")] {
            log.push_input(Input {
                channel: None,
                id: i,
                effect: effect.to_string(),
                timestamp: Timestamp::now(),
                data: serde_json::json!(i),
                metadata: vec![],
            })
            .unwrap();
        }

        assert_eq!(log.last_for_effect("a").unwrap().id, 3);
        assert_eq!(log.last_for_effect("b").unwrap().id, 2);
        assert!(log.last_for_effect("c").is_none());
    }

    fn rng(seed: u64) -> Rng {
        Rng::from_seed(&format!("{seed}-log"))
    }

    /// Builds a `SqliteLog`, populates it with ten inputs on effect "a", then draws
    /// `random_for_effect` `draws` times under an rng seeded from `seed`, returning the sampled
    /// ids.
    fn sampled_ids(seed: u64, draws: usize) -> Vec<u64> {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();
        let mut rng = rng(seed);

        for i in 1..=10u64 {
            log.push_input(Input {
                channel: None,
                id: i,
                effect: "a".to_string(),
                timestamp: Timestamp::now(),
                data: serde_json::json!(i),
                metadata: vec![],
            })
            .unwrap();
        }

        (0..draws)
            .map(|_| log.random_for_effect("a", &mut rng).unwrap().id)
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
        let log = SqliteLog::in_dir(tmp.path()).unwrap();
        let mut rng = rng(1);

        assert!(log.random_for_effect("nonexistent", &mut rng).is_none());
    }

    fn push_inputs(log: &SqliteLog, effect: &str, count: u64) {
        for i in 1..=count {
            log.push_input(Input {
                channel: None,
                id: i,
                effect: effect.to_string(),
                timestamp: Timestamp::now(),
                data: serde_json::json!(i),
                metadata: vec![],
            })
            .unwrap();
        }
    }

    #[test]
    fn unique_cursor_never_repeats_and_exhausts() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();
        push_inputs(&log, "a", 5);
        let mut rng = rng(1);

        let mut seen = std::collections::HashSet::new();
        for _ in 0..5 {
            let sampled = log.unique_for_effect("a", "cursor", &mut rng).unwrap();
            assert!(
                seen.insert(sampled.id),
                "id {} returned more than once",
                sampled.id
            );
        }

        assert!(log.unique_for_effect("a", "cursor", &mut rng).is_none());
    }

    #[test]
    fn unique_cursor_is_deterministic_for_a_fixed_seed() {
        fn draw_all(seed: u64) -> Vec<u64> {
            let tmp = TempDir::new().unwrap();
            let log = SqliteLog::in_dir(tmp.path()).unwrap();
            push_inputs(&log, "a", 10);
            let mut rng = rng(seed);

            std::iter::from_fn(|| log.unique_for_effect("a", "cursor", &mut rng).map(|e| e.id))
                .collect()
        }

        assert_eq!(draw_all(42), draw_all(42));
    }

    #[test]
    fn unique_cursor_state_is_independent_per_cursor() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();
        push_inputs(&log, "a", 1);
        let mut rng = rng(1);

        assert_eq!(
            log.unique_for_effect("a", "cursor-a", &mut rng).unwrap().id,
            1
        );
        // A second, independent cursor over the same effect can still draw the same input.
        assert_eq!(
            log.unique_for_effect("a", "cursor-b", &mut rng).unwrap().id,
            1
        );
    }

    #[test]
    fn unique_cursor_consumed_markers_do_not_leak_into_input_metadata() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();
        push_inputs(&log, "a", 1);
        let mut rng = rng(1);

        log.unique_for_effect("a", "cursor", &mut rng).unwrap();

        let last = log.last().unwrap();
        assert!(last.metadata.is_empty());
    }

    #[test]
    fn unique_cursor_state_survives_reopening_the_log() {
        let tmp = TempDir::new().unwrap();
        let mut rng = rng(1);
        let mut seen = std::collections::HashSet::new();

        let log = SqliteLog::in_dir(tmp.path()).unwrap();
        push_inputs(&log, "a", 6);
        for _ in 0..3 {
            let sampled = log.unique_for_effect("a", "cursor", &mut rng).unwrap();
            seen.insert(sampled.id);
        }
        drop(log);

        let log = SqliteLog::in_dir(tmp.path()).unwrap();
        for _ in 0..3 {
            let sampled = log.unique_for_effect("a", "cursor", &mut rng).unwrap();
            assert!(
                seen.insert(sampled.id),
                "id {} returned more than once",
                sampled.id
            );
        }
        assert!(log.unique_for_effect("a", "cursor", &mut rng).is_none());
    }

    #[test]
    fn get_signal_returns_query_value() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();
        push_inputs(&log, "a", 3);
        log.commit().unwrap();

        let value = log.query("SELECT COUNT(*) FROM inputs");

        assert_eq!(value.unwrap(), serde_json::json!(3));
    }

    #[test]
    fn get_signal_returns_none_on_query_error() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteLog::in_dir(tmp.path()).unwrap();

        let value = log.query("SELECT COUNT(*) FROM missing_table");

        let Err(error @ LogError::Sqlite(_)) = value else {
            panic!("expected a sqlite error, got {value:?}");
        };
        let source = std::error::Error::source(&error).unwrap();
        assert!(source.downcast_ref::<rusqlite::Error>().is_some());
        assert!(
            error.to_string().starts_with("sqlite: no such table"),
            "{error}"
        );
    }
}
