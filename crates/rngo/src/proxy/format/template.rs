use crate::error::{BuildError, BuildErrors, ParseErrors};
use handlebars::{
    Context, Handlebars, Helper, HelperResult, Output, RenderContext, RenderError,
    RenderErrorReason,
};

use serde_json::json;

use crate::effect::Input;
use crate::proxy::format::{
    Format, FormatBuildContext, FormatBuilder, FormatContext, FormatParseContext, FormatParser,
};
use crate::{FormatError, ParseError, spec};

const TEMPLATE_NAME: &str = "template";

/// Formats each event by rendering a Handlebars template against the whole `Input` (with the
/// effect exposed as `effect.key`), without HTML escaping and with a `json` helper that serializes its one argument.
#[derive(Debug)]
pub(crate) struct TemplateFormat {
    hbs: Handlebars<'static>,
}

impl TemplateFormat {
    pub(crate) fn parser() -> TemplateFormatParser {
        TemplateFormatParser {}
    }

    pub(crate) fn builder() -> TemplateFormatBuilder {
        TemplateFormatBuilder { template: None }
    }

    /// Compiles `template`, failing if it is invalid.
    fn new(template: &str) -> Result<Self, FormatError> {
        let mut hbs = Handlebars::new();
        hbs.register_escape_fn(handlebars::no_escape);
        hbs.register_helper("json", Box::new(json_helper));
        hbs.register_template_string(TEMPLATE_NAME, template)
            .map_err(|e| FormatError::Template(e.to_string()))?;
        Ok(TemplateFormat { hbs })
    }
}

fn json_helper(
    h: &Helper,
    _: &Handlebars,
    _: &Context,
    _: &mut RenderContext,
    out: &mut dyn Output,
) -> HelperResult {
    if h.params().len() != 1 {
        return Err(RenderErrorReason::Other(format!(
            "json helper takes exactly one argument, got {}",
            h.params().len()
        ))
        .into());
    }

    let json = serde_json::to_string(h.param(0).unwrap().value())
        .map_err(|e| RenderError::from(RenderErrorReason::Other(e.to_string())))?;
    out.write(&json)?;
    Ok(())
}

impl Format for TemplateFormat {
    fn format(&self, event: &Input, _context: &FormatContext) -> Result<String, FormatError> {
        let mut context = serde_json::to_value(event)?;
        context["effect"] = json!({ "key": event.effect });
        self.hbs
            .render(TEMPLATE_NAME, &context)
            .map_err(|e| FormatError::Render(e.to_string()))
    }
}

/// Builds the `template` format, which renders each input through a Handlebars template.
#[derive(Debug)]
pub struct TemplateFormatBuilder {
    template: Option<String>,
}

impl TemplateFormatBuilder {
    /// The Handlebars template, rendered against the whole input; required, and compiled at build.
    pub fn template(mut self, template: impl Into<String>) -> Self {
        self.template = Some(template.into());
        self
    }
}

impl FormatBuilder for TemplateFormatBuilder {
    fn build(self: Box<Self>, context: FormatBuildContext) -> Result<Box<dyn Format>, BuildErrors> {
        let channel_key = context.channel_key();
        let Some(template) = self.template else {
            return Err(BuildError::channel(channel_key, "format: template is required").into());
        };
        let format = TemplateFormat::new(&template)
            .map_err(|e| BuildError::channel(channel_key, format!("format: {e}")))?;
        Ok(Box::new(format))
    }
}

pub(crate) struct TemplateFormatParser;

impl FormatParser for TemplateFormatParser {
    fn key(&self) -> &str {
        "template"
    }

    fn parse(
        &self,
        format: &spec::Format,
        _context: &FormatParseContext,
    ) -> Result<Box<dyn FormatBuilder>, ParseErrors> {
        let error =
            |message: String| ParseErrors::from(ParseError::invalid(message).at(["template"]));

        let template = match format.fields.get("template") {
            Some(value) => value
                .as_str()
                .ok_or_else(|| error("must be a string".into()))?,
            None => return Err(error("template is required".into())),
        };

        let format = TemplateFormat::new(template).map_err(|e| error(e.to_string()))?;

        Ok(Box::new(format))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Timestamp;
    use serde_json::Value;

    fn event(data: Value) -> Input {
        Input {
            id: 7,
            effect: "user".to_string(),
            timestamp: Timestamp::now(),
            data,
            metadata: vec![],
        }
    }

    fn render(template: &str, data: Value) -> Result<String, FormatError> {
        TemplateFormat::new(template)
            .unwrap()
            .format(&event(data), &FormatContext::new())
    }

    fn parse(format: Value) -> Result<Box<dyn Format>, ParseErrors> {
        let format: spec::Format = serde_json::from_value(format).unwrap();
        let spec: spec::Spec = serde_json::from_value(json!({ "effects": {} })).unwrap();
        let builder = TemplateFormat::parser().parse(&format, &FormatParseContext::new(&spec))?;
        Ok(builder.build(FormatBuildContext::new("out")).unwrap())
    }

    #[test]
    fn renders_against_the_whole_event() {
        let output = render(
            "{{effect.key}} {{id}} {{data.name}}",
            json!({ "name": "alice" }),
        );
        assert_eq!(output.unwrap(), "user 7 alice");
    }

    #[test]
    fn does_not_escape_html() {
        let output = render("{{data.s}}", json!({ "s": "<a> & \"b\" 'c'" }));
        assert_eq!(output.unwrap(), "<a> & \"b\" 'c'");
    }

    #[test]
    fn json_helper_serializes_every_value_kind() {
        let data = json!({
            "object": { "a": 1 },
            "array": [1, "two"],
            "string": "it's \"quoted\"",
            "number": 1.5,
            "bool": true,
            "null": null
        });
        let cases = [
            ("{{json data.object}}", r#"{"a":1}"#),
            ("{{json data.array}}", r#"[1,"two"]"#),
            ("{{json data.string}}", r#""it's \"quoted\"""#),
            ("{{json data.number}}", "1.5"),
            ("{{json data.bool}}", "true"),
            ("{{json data.null}}", "null"),
        ];
        for (template, expected) in cases {
            assert_eq!(render(template, data.clone()).unwrap(), expected);
        }
    }

    #[test]
    fn json_helper_errors_on_bad_arguments() {
        assert!(render("{{json}}", json!({})).is_err());
        assert!(render("{{json data id}}", json!({})).is_err());
    }

    #[test]
    fn builder_builds_a_format() {
        let format = Box::new(TemplateFormat::builder().template("{{effect.key}}"))
            .build(FormatBuildContext::new("out"))
            .unwrap();
        assert_eq!(
            format
                .format(&event(json!(null)), &FormatContext::new())
                .unwrap(),
            "user"
        );
    }

    #[test]
    fn builder_errors_on_a_missing_or_invalid_template() {
        let missing = Box::new(TemplateFormat::builder())
            .build(FormatBuildContext::new("out"))
            .unwrap_err();
        assert!(missing[0].to_string().contains("template is required"));
        let invalid = Box::new(TemplateFormat::builder().template("{{#if}}"))
            .build(FormatBuildContext::new("out"))
            .unwrap_err();
        assert_eq!(invalid.len(), 1);
    }

    #[test]
    fn parser_builds_a_format() {
        let format = parse(json!({ "type": "template", "template": "{{effect.key}}" })).unwrap();
        assert_eq!(
            format
                .format(&event(json!(null)), &FormatContext::new())
                .unwrap(),
            "user"
        );
    }

    #[test]
    fn parser_errors_on_a_missing_template() {
        let errors = parse(json!({ "type": "template" })).unwrap_err();
        assert!(matches!(
            &errors[..],
            [e] if e.path() == ["template"]
        ));
    }

    #[test]
    fn parser_errors_on_a_non_string_template() {
        let errors = parse(json!({ "type": "template", "template": 1 })).unwrap_err();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn parser_errors_on_an_invalid_template() {
        let errors = parse(json!({ "type": "template", "template": "{{#if}}" })).unwrap_err();
        assert_eq!(errors.len(), 1);
    }
}
