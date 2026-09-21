use crate::util::json_pointer::{JsonPointer, JsonPointerPart};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(rename = "type")]
    pub mtype: String,
    pub attribute: Option<JsonPointer>,
    pub data: Option<Value>,
}

impl Metadata {
    pub fn prefix_attribute(&mut self, part: JsonPointerPart) {
        if let Some(attribute) = &mut self.attribute {
            attribute.prefix(part)
        } else {
            self.attribute = Some(part.into())
        }
    }
}
