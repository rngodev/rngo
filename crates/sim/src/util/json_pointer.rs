use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::VecDeque;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone)]
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

impl FromStr for JsonPointer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Ok(JsonPointer {
                parts: VecDeque::new(),
            });
        }

        if !s.starts_with('/') {
            return Err(format!("invalid JSON pointer: {s}"));
        }

        let parts = s[1..]
            .split('/')
            .map(|segment| {
                let unescaped = segment.replace("~1", "/").replace("~0", "~");
                match unescaped.parse::<u32>() {
                    Ok(index) => JsonPointerPart::Index(index),
                    Err(_) => JsonPointerPart::Field(unescaped),
                }
            })
            .collect();

        Ok(JsonPointer { parts })
    }
}

impl Serialize for JsonPointer {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for JsonPointer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(DeError::custom)
    }
}

impl From<JsonPointerPart> for JsonPointer {
    fn from(value: JsonPointerPart) -> Self {
        let mut parts = VecDeque::new();
        parts.push_front(value);
        JsonPointer { parts }
    }
}

#[derive(Debug, Clone)]
pub enum JsonPointerPart {
    Field(String),
    Index(u32),
}

impl From<String> for JsonPointerPart {
    fn from(value: String) -> Self {
        JsonPointerPart::Field(value)
    }
}
