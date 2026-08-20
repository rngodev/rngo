use std::collections::VecDeque;

#[derive(Clone)]
pub struct JsonPointer {
    parts: VecDeque<JsonPointerPart>,
}

impl JsonPointer {
    pub fn prefix(&mut self, part: JsonPointerPart) {
        self.parts.push_front(part)
    }
}

impl From<JsonPointerPart> for JsonPointer {
    fn from(value: JsonPointerPart) -> Self {
        let mut parts = VecDeque::new();
        parts.push_front(value);
        JsonPointer { parts }
    }
}

#[derive(Clone)]
pub enum JsonPointerPart {
    Field(String),
    Index(u32),
}

impl From<String> for JsonPointerPart {
    fn from(value: String) -> Self {
        JsonPointerPart::Field(value)
    }
}

impl From<String> for JsonPointerPart {
    fn from(value: String) -> Self {
        JsonPointerPart::Field(value)
    }
}
