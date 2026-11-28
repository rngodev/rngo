use crate::effect::Input;
use crate::error::{BuildErrors, ParseErrors};
use crate::proxy::format::{
    Format, FormatBuildContext, FormatBuilder, FormatContext, FormatParseContext, FormatParser,
};
use crate::{FormatError, spec};
use serde_json::Value;

#[derive(Debug)]
pub(crate) struct CsvFormat {}

impl CsvFormat {
    pub(crate) fn parser() -> CsvFormatParser {
        CsvFormatParser {}
    }

    pub(crate) fn builder() -> CsvFormatBuilder {
        CsvFormatBuilder::default()
    }
}

/// Builds the `csv` format, which renders an input's `data` object as one CSV row with no header,
/// one field per key in key order.
#[derive(Debug, Default)]
pub struct CsvFormatBuilder {
    _private: (),
}

impl FormatBuilder for CsvFormatBuilder {
    fn build(
        self: Box<Self>,
        _context: FormatBuildContext,
    ) -> Result<Box<dyn Format>, BuildErrors> {
        Ok(Box::new(CsvFormat {}))
    }
}

impl Format for CsvFormat {
    fn format(&self, input: &Input, _context: &FormatContext) -> Result<String, FormatError> {
        let Value::Object(map) = &input.data else {
            return Err(FormatError::Render(
                "the csv format needs an object, one column per key".into(),
            ));
        };
        let fields = map
            .values()
            .map(|value| match value {
                Value::Null => String::new(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                Value::String(s) => quote(s),
                other => quote(&other.to_string()),
            })
            .collect::<Vec<_>>();
        Ok(fields.join(","))
    }
}

/// `field` as a quoted CSV field, with its quotes doubled.
fn quote(field: &str) -> String {
    format!("\"{}\"", field.replace('"', "\"\""))
}

pub(crate) struct CsvFormatParser {}

impl FormatParser for CsvFormatParser {
    fn key(&self) -> &str {
        "csv"
    }

    fn parse(
        &self,
        _format: &spec::Format,
        _context: &FormatParseContext,
    ) -> Result<Box<dyn FormatBuilder>, ParseErrors> {
        Ok(Box::new(CsvFormat::builder()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Timestamp;
    use serde_json::json;

    fn format(data: Value) -> Result<String, FormatError> {
        CsvFormat {}.format(
            &Input::new(1, "user", Timestamp::now(), data),
            &FormatContext::new(),
        )
    }

    #[test]
    fn writes_one_field_per_key_in_key_order() {
        let row = format(json!({ "b": 1, "a": 2.5, "c": true })).unwrap();
        assert_eq!(row, "2.5,1,true");
    }

    #[test]
    fn null_is_an_empty_field_and_strings_are_always_quoted() {
        let row = format(json!({ "a": null, "b": "", "c": "x" })).unwrap();
        assert_eq!(row, r#","","x""#);
    }

    #[test]
    fn quotes_are_doubled_and_commas_and_newlines_stay_inside_quotes() {
        let row = format(json!({ "a": "say \"hi\", then\nbye" })).unwrap();
        assert_eq!(row, "\"say \"\"hi\"\", then\nbye\"");
    }

    #[test]
    fn arrays_and_objects_are_quoted_json() {
        let row = format(json!({ "a": [1, "x"], "b": { "k": "v" } })).unwrap();
        assert_eq!(row, r#""[1,""x""]","{""k"":""v""}""#);
    }

    #[test]
    fn data_that_isnt_an_object_is_a_format_error() {
        for data in [json!(1), json!("x"), json!([1]), json!(null)] {
            assert!(format(data).is_err());
        }
    }
}
