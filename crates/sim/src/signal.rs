use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Io {
    Stdout,
    Stderr,
}

#[derive(Debug, Serialize)]
pub struct Signal {
    pub effect: Option<String>,
    pub system: String,
    pub io: Io,
    pub data: String,
}
