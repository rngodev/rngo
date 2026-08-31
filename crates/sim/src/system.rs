use std::collections::HashMap;

use crate::Channel;

pub struct System {
    pub channels: Vec<Channel>,
    pub effect_channels: HashMap<String, String>,
}
