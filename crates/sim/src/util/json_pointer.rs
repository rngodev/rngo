use std::collections::VecDeque;
use std::fmt;

#[derive(Clone)]
pub struct JsonPointer {
    parts: VecDeque<JsonPointerPart>,
}

impl JsonPointer {
    pub fn prefix(&mut self, part: JsonPointerPart) {
        self.parts.push_front(part)
    }
}

impl fmt::Display for JsonPointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for part in &self.parts {
            write!(f, "/")?;
            match part {
                JsonPointerPart::Field(field) => {
                    write!(f, "{}", field.replace('~', "~0").replace('/', "~1"))?
                }
                JsonPointerPart::Index(index) => write!(f, "{index}")?,
            }
        }
        Ok(())
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
