use thiserror::Error;

#[derive(Error, Debug)]
#[error("failed to build: `{message}`")]
pub enum BuildError {
    Simulation {
        key: SimulationKey,
        message: String,
    },
    Effect {
        effect: String,
        key: EffectKey,
        message: String,
    },
    Schema {
        effect: String,
        path: Vec<SchemaEdge>,
        message: String,
    },
    Proxy {
        message: String,
    },
    Channel {
        channel: String,
        message: String,
    },
    ChannelTarget {
        channel: String,
        message: String,
    },
    Signal {
        signal: String,
        message: String,
    },
    Audit {
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct SchemaEdge {
    pub kind: &'static str,
    pub key: String,
}

#[derive(Debug)]
pub enum SimulationKey {
    Start,
    End,
}

#[derive(Debug)]
pub enum EffectKey {
    Schema,
    Trigger,
    Config,
    Start,
    End,
}
