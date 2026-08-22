use crate::log::LogReader;
use crate::output::Level;
use crate::schema::Metadata;
use crate::{Log, LogEvent};
use rusqlite::Connection;
use std::path::PathBuf;
use std::rc::Rc;

/// Number of pushed events to accumulate in a single transaction before committing.
const BATCH_SIZE: usize = 500;

#[derive(Debug)]
pub struct SqliteProxyLog {
    child: Box<dyn Log>,
    connection: Connection,
    pending: usize,
}

impl SqliteProxyLog {
    pub fn new(child: Box<dyn Log>, directory: PathBuf) -> Self {
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

        SqliteProxyLog {
            child,
            connection,
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
            self.connection.execute_batch("COMMIT; BEGIN;").unwrap();
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
        for m in metadata {
            self.connection
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

impl Log for SqliteProxyLog {
    fn push(&mut self, event: LogEvent) {
        match &event {
            LogEvent::Input(e) => {
                self.connection
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
        self.child.push(event);
    }

    fn reader(&self) -> Rc<dyn LogReader> {
        self.child.reader()
    }
}

impl Drop for SqliteProxyLog {
    fn drop(&mut self) {
        let _ = self.connection.execute_batch("COMMIT;");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{Input, SkippedInput};
    use crate::log::SimpleEventLog;
    use crate::output::Output;
    use chrono::Utc;
    use tempfile::TempDir;

    fn open(directory: &std::path::Path) -> Connection {
        Connection::open(directory.join("log.sqlite")).unwrap()
    }

    #[test]
    fn writes_input_output_and_metadata_rows() {
        let tmp = TempDir::new().unwrap();
        let mut log = SqliteProxyLog::new(
            Box::new(SimpleEventLog::default()),
            tmp.path().to_path_buf(),
        );

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
        log.push(LogEvent::Output(Output {
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
        let mut log = SqliteProxyLog::new(
            Box::new(SimpleEventLog::default()),
            tmp.path().to_path_buf(),
        );

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
        let mut log = SqliteProxyLog::new(
            Box::new(SimpleEventLog::default()),
            tmp.path().to_path_buf(),
        );

        for i in 0..(BATCH_SIZE * 2 + 3) {
            log.push(LogEvent::Input(Input {
                id: i as u64,
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
}
