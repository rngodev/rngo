use crate::log::LogReader;
use crate::signal::Level;
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

                CREATE TABLE IF NOT EXISTS effects (
                    id INTEGER NOT NULL,
                    key TEXT NOT NULL,
                    offset INTEGER NOT NULL,
                    value TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS signals (
                    effect_id INTEGER,
                    timestamp TEXT NOT NULL,
                    system TEXT NOT NULL,
                    level TEXT NOT NULL,
                    data TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS errors (
                    message TEXT NOT NULL
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
}

impl Log for SqliteProxyLog {
    fn push(&mut self, event: LogEvent) {
        match &event {
            LogEvent::Effect(e) => {
                self.connection
                    .prepare_cached(
                        "INSERT INTO effects (id, key, offset, value) VALUES (?1, ?2, ?3, ?4)",
                    )
                    .unwrap()
                    .execute(rusqlite::params![
                        e.id as i64,
                        e.key,
                        e.offset as i64,
                        serde_json::to_string(&e.value).unwrap(),
                    ])
                    .unwrap();
            }
            LogEvent::Signal(s) => {
                self.connection
                    .prepare_cached(
                        "INSERT INTO signals (effect_id, timestamp, system, level, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                    )
                    .unwrap()
                    .execute(rusqlite::params![
                        s.effect_id.map(|id| id as i64),
                        s.timestamp.to_rfc3339(),
                        s.system,
                        match s.level {
                            Level::Error => "error",
                            Level::Warning => "warning",
                            Level::Info => "info",
                        },
                        s.data,
                    ])
                    .unwrap();
            }
            LogEvent::Error(message) => {
                self.connection
                    .prepare_cached("INSERT INTO errors (message) VALUES (?1)")
                    .unwrap()
                    .execute(rusqlite::params![message])
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
    use crate::effect::EffectEvent;
    use crate::log::SimpleEventLog;
    use crate::signal::Signal;
    use chrono::Utc;
    use tempfile::TempDir;

    fn open(directory: &std::path::Path) -> Connection {
        Connection::open(directory.join("log.sqlite")).unwrap()
    }

    #[test]
    fn writes_effect_signal_and_error_rows() {
        let tmp = TempDir::new().unwrap();
        let mut log = SqliteProxyLog::new(
            Box::new(SimpleEventLog::default()),
            tmp.path().to_path_buf(),
        );

        log.push(LogEvent::Effect(EffectEvent {
            id: 1,
            key: "ping".to_string(),
            offset: 42,
            timestamp: Utc::now().fixed_offset(),
            value: serde_json::json!({ "a": 1 }),
        }));
        log.push(LogEvent::Signal(Signal {
            effect_id: Some(1),
            timestamp: Utc::now(),
            system: "logger".to_string(),
            level: Level::Info,
            data: "hello".to_string(),
        }));
        log.push(LogEvent::Error("boom".to_string()));

        // Force the pending transaction closed so the rows are visible to a fresh connection.
        log.commit();

        let conn = open(tmp.path());

        let effect_key: String = conn
            .query_row("SELECT key FROM effects", [], |row| row.get(0))
            .unwrap();
        assert_eq!(effect_key, "ping");

        let signal_data: String = conn
            .query_row("SELECT data FROM signals", [], |row| row.get(0))
            .unwrap();
        assert_eq!(signal_data, "hello");

        let error_message: String = conn
            .query_row("SELECT message FROM errors", [], |row| row.get(0))
            .unwrap();
        assert_eq!(error_message, "boom");
    }

    #[test]
    fn batches_across_commits() {
        let tmp = TempDir::new().unwrap();
        let mut log = SqliteProxyLog::new(
            Box::new(SimpleEventLog::default()),
            tmp.path().to_path_buf(),
        );

        for i in 0..(BATCH_SIZE * 2 + 3) {
            log.push(LogEvent::Effect(EffectEvent {
                id: i as u64,
                key: "ping".to_string(),
                offset: i as u64,
                timestamp: Utc::now().fixed_offset(),
                value: serde_json::json!(i),
            }));
        }
        log.commit();

        let conn = open(tmp.path());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM effects", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, (BATCH_SIZE * 2 + 3) as i64);
    }
}
