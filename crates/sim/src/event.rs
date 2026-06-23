mod log;

pub use log::{EventLog, EventLogIndex, EventLogIndexConfig, SimpleEventLog};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Effect {
        id: u64,
        key: String,
        offset: u64,
        value: Value,
        format: Option<String>,
    },
    Error {
        id: u64,
        message: String,
    },
}

impl Event {
    pub fn id(&self) -> u64 {
        let id = match self {
            Event::Effect { id, .. } => id,
            Event::Error { id, .. } => id,
        };

        id.to_owned()
    }

    pub fn value(&self) -> Option<&Value> {
        match self {
            Event::Effect { value, .. } => Some(value),
            Event::Error { .. } => None,
        }
    }
}
