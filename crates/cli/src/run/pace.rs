use super::status::StatusWriter;
use chrono::{DateTime, FixedOffset, Utc};
use rngo::{Pacer, RunLogWriter};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

const WAIT_STEP: Duration = Duration::from_millis(100);

/// Holds each input until the wall clock reaches its timestamp, or until `stop` is set.
pub struct Realtime {
    pub stop: Arc<AtomicBool>,
    pub status: Rc<StatusWriter>,
}

impl Pacer for Realtime {
    fn wait_until(&mut self, timestamp: DateTime<FixedOffset>) -> bool {
        let mut flushed = false;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                self.status.wait(None);
                return false;
            }

            let Ok(remaining) = (timestamp.to_utc() - Utc::now()).to_std() else {
                self.status.wait(None);
                return true;
            };

            if !flushed {
                self.status.flush();
                flushed = true;
            }

            self.status.wait(Some(timestamp));
            thread::sleep(remaining.min(WAIT_STEP));
        }
    }
}
