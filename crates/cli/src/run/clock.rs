use rngo::{Metadata, RunLogWriter, SqliteRunLog};
use std::rc::Rc;

pub struct RunClock {
    log: Rc<SqliteRunLog>,
    ended: bool,
}

impl RunClock {
    pub fn start(log: Rc<SqliteRunLog>) -> Self {
        record(&log, "run_start");
        RunClock { log, ended: false }
    }

    pub fn end(&mut self) {
        if !self.ended {
            record(&self.log, "run_end");
            self.ended = true;
        }
    }
}

impl Drop for RunClock {
    fn drop(&mut self) {
        self.end();
    }
}

fn record(log: &SqliteRunLog, mtype: &str) {
    log.push_metadata(Metadata {
        mtype: mtype.to_string(),
        input_id: None,
        output_id: None,
        offset: None,
        data: Some(chrono::Utc::now().to_rfc3339().into()),
        segment: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn records_end_when_dropped_without_ending() {
        let tmp = TempDir::new().unwrap();
        let log = SqliteRunLog::new(tmp.path().to_path_buf());

        drop(RunClock::start(log.clone()));
        drop(log);

        let connection = rusqlite::Connection::open(tmp.path().join("log.sqlite")).unwrap();
        let types: Vec<String> = connection
            .prepare("SELECT type FROM metadata ORDER BY rowid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|t| t.unwrap())
            .collect();
        assert_eq!(types, ["run_start", "run_end"]);
    }
}
