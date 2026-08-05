mod sql;

use crate::effect::EffectEvent;
use std::fmt::Debug;

pub use sql::SqlFormat;

pub trait Format: Debug {
    fn format(&self, event: &EffectEvent) -> Result<String, String>;
}
