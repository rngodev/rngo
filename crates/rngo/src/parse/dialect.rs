use super::format::{FormatContext, FormatParser};
use super::generator::{GeneratorParseVisitor, GeneratorParser};
use super::signal::SignalParser;
use crate::audit::signal::Signal;
use crate::audit::{Audit, AuditBuilder, signal};
use crate::effect::generator::custom::CustomParser;
use crate::effect::simulation::{Simulation, SimulationBuilder};
use crate::effect::{Effect, generator};
use crate::moment::Moment;
use crate::parse::ChannelParser;
use crate::proxy::channel::{self, ChannelBuilder};
use crate::proxy::{Proxy, ProxyBuilder, format};
use crate::spec::{self, ParseError, Spec};
use std::collections::HashMap;
use std::rc::Rc;

pub struct Dialect {
    generator_parsers: Rc<Vec<Box<dyn GeneratorParser>>>,
    format_parsers: Rc<Vec<Box<dyn FormatParser>>>,
    channel_parsers: Rc<Vec<Box<dyn ChannelParser>>>,
    signal_parsers: Rc<Vec<Box<dyn SignalParser>>>,
}

impl Dialect {
    pub fn new(
        generator_parsers: Vec<Box<dyn GeneratorParser>>,
        format_parsers: Vec<Box<dyn FormatParser>>,
        channel_parsers: Vec<Box<dyn ChannelParser>>,
        signal_parsers: Vec<Box<dyn SignalParser>>,
    ) -> Self {
        Dialect {
            generator_parsers: Rc::new(generator_parsers),
            format_parsers: Rc::new(format_parsers),
            channel_parsers: Rc::new(channel_parsers),
            signal_parsers: Rc::new(signal_parsers),
        }
    }

    pub fn primitive() -> Self {
        Dialect::new(
            vec![
                Box::new(generator::Array::parser()),
                Box::new(generator::Constant::parser()),
                Box::new(generator::Context::parser()),
                Box::new(generator::Function::parser()),
                Box::new(generator::Number::parser()),
                Box::new(generator::Object::parser()),
                Box::new(generator::Reference::parser()),
                Box::new(generator::Select::parser()),
                Box::new(generator::Str::parser()),
            ],
            vec![
                Box::new(format::SqlFormat::parser()),
                Box::new(format::TemplateFormat::parser()),
            ],
            vec![
                Box::new(channel::Exec::parser()),
                Box::new(channel::Stdout::parser()),
                Box::new(channel::Stream::parser()),
            ],
            vec![Box::new(signal::SqlSignal::parser())],
        )
    }

    pub fn parse_simulation_json(
        &self,
        value: serde_json::Value,
    ) -> Result<SimulationBuilder, Vec<ParseError>> {
        let spec: Spec = spec::from_value(value)?;
        self.parse_simulation(spec)
    }

    pub fn parse_simulation(&self, spec: Spec) -> Result<SimulationBuilder, Vec<ParseError>> {
        let mut errors = vec![];
        let mut simulation_builder = Simulation::builder();
        let simulation_moment_parser = Moment::parser();

        if let Some(seed) = spec.seed {
            simulation_builder.set_seed(seed);
        }

        if let Some(start) = &spec.start {
            match simulation_moment_parser.parse("start", start) {
                Ok(timestamp) => {
                    simulation_builder.set_start(timestamp);
                }
                Err(mut e) => errors.append(&mut e),
            };
        };

        if let Some(end) = &spec.end {
            match simulation_moment_parser.parse("end", end) {
                Ok(timestamp) => {
                    simulation_builder.set_end(timestamp);
                }
                Err(mut e) => errors.append(&mut e),
            };
        };

        for name in spec.generators.keys() {
            if self.generator_parsers.iter().any(|p| p.key() == name) {
                errors.push(ParseError::Invalid {
                    path: Some(vec!["generators".into(), name.clone()]),
                    message: format!(
                        "\"{name}\" is a primitive generator type and cannot be used as a custom generator name"
                    ),
                });
            }
        }

        let custom_generators: Rc<Vec<CustomParser>> = Rc::new(
            spec.generators
                .iter()
                .map(|(name, generator_type)| {
                    CustomParser::new(name.clone(), generator_type.clone())
                })
                .collect(),
        );

        for (key, effect) in &spec.effects {
            let mut effect_builder = Effect::builder(key.clone());
            let effect_moment_parser =
                Moment::parser().simulation(&simulation_builder.start, &simulation_builder.end);

            if let Some(start) = &effect.start {
                match effect_moment_parser.parse("start", start) {
                    Ok(timestamp) => {
                        effect_builder.set_start(timestamp);
                    }
                    Err(mut e) => errors.append(&mut e),
                };
            };

            if let Some(end) = &effect.end {
                match effect_moment_parser.parse("end", end) {
                    Ok(timestamp) => {
                        effect_builder.set_end(timestamp);
                    }
                    Err(mut e) => errors.append(&mut e),
                };
            };

            if let Some(limit) = effect.limit {
                effect_builder.set_limit(limit);
            }

            if let Some(trigger_union) = &effect.trigger {
                let trigger = match trigger_union {
                    spec::TriggerUnion::Shorthand(rate) => {
                        spec::Trigger::Clock { rate: rate.clone() }
                    }
                    spec::TriggerUnion::Full(trigger) => trigger.clone(),
                };

                match trigger {
                    spec::Trigger::Clock { rate } => effect_builder.set_trigger_expression(rate),
                    spec::Trigger::Effect { key } => effect_builder.set_trigger_effect(key),
                };
            }

            let visitor = GeneratorParseVisitor::new(
                self.generator_parsers.clone(),
                Rc::clone(&custom_generators),
                effect.generator.clone(),
                vec![],
                vec!["effects".into(), key.clone(), "generator".into()],
            );

            match visitor.parse() {
                Ok(generator_builder) => {
                    effect_builder.set_generator(generator_builder);
                    simulation_builder.set_effect(effect_builder);
                }
                Err(mut e) => errors.append(&mut e),
            }
        }

        if !errors.is_empty() {
            Err(errors)
        } else {
            Ok(simulation_builder)
        }
    }

    pub fn parse_proxy(&self, spec: Spec) -> Result<ProxyBuilder, Vec<ParseError>> {
        let mut errors = vec![];
        let mut proxy_builder = Proxy::builder();

        let effect_channels: HashMap<String, String> = spec
            .effects
            .iter()
            .filter_map(|(k, v)| v.channel.as_ref().map(|s| (k.clone(), s.clone())))
            .collect();

        for (effect_key, channel_key) in &effect_channels {
            if !spec.channels.keys().any(|key| key == channel_key) {
                errors.push(ParseError::Invalid {
                    path: Some(vec!["effects".into(), effect_key.clone(), "channel".into()]),
                    message: format!("unknown channel: {channel_key}"),
                });
            }
        }

        let formats = FormatContext {
            parsers: &self.format_parsers,
            spec: &spec,
        };

        for (key, channel) in &spec.channels {
            if channel.fields.contains_key("target") {
                errors.push(ParseError::Invalid {
                    path: Some(vec!["channels".into(), key.clone(), "target".into()]),
                    message: "channels no longer take a target; set `type` (and e.g. `command`) \
                              directly on the channel"
                        .into(),
                });
            } else {
                match self.parse_channel(channel, &formats) {
                    Ok(channel_builder) => {
                        proxy_builder.set_channel(key.clone(), channel_builder);
                    }
                    Err(e) => errors.extend(prefix_paths(e, &["channels", key])),
                }
            }

            let effects: Vec<_> = effect_channels
                .iter()
                .filter(|(_, channel_key)| *channel_key == key)
                .map(|(effect_key, _)| effect_key.as_str())
                .collect();

            proxy_builder.set_channel_effects(key.clone(), &effects);
        }

        if !errors.is_empty() {
            Err(errors)
        } else {
            Ok(proxy_builder)
        }
    }

    pub fn parse_audit(&self, spec: Spec) -> Result<AuditBuilder, Vec<ParseError>> {
        let mut errors = vec![];
        let mut audit_builder = Audit::builder();

        for (key, signal) in &spec.signals {
            match self.parse_signal(key, signal) {
                Ok(built) => {
                    audit_builder.set_signal(key.clone(), built);
                }
                Err(mut e) => errors.append(&mut e),
            }
        }

        if !errors.is_empty() {
            Err(errors)
        } else {
            Ok(audit_builder)
        }
    }

    fn parse_signal(
        &self,
        key: &str,
        signal: &spec::Signal,
    ) -> Result<Box<dyn Signal>, Vec<ParseError>> {
        let matching: Vec<_> = self
            .signal_parsers
            .iter()
            .filter(|p| signal.stype.as_deref() == Some(p.key()))
            .collect();

        match matching.as_slice() {
            [parser] => parser.parse(key, signal),
            [] => Err(vec![ParseError::Invalid {
                path: None,
                message: "unknown signal type".to_string(),
            }]),
            _ => Err(vec![ParseError::Invalid {
                path: None,
                message: format!("{} signal parsers matched", matching.len()),
            }]),
        }
    }

    fn parse_channel(
        &self,
        channel: &spec::Channel,
        formats: &FormatContext,
    ) -> Result<Box<dyn ChannelBuilder>, Vec<ParseError>> {
        let matching: Vec<_> = self
            .channel_parsers
            .iter()
            .filter(|p| channel.ctype.as_deref() == Some(p.key()))
            .collect();

        match matching.as_slice() {
            [parser] => parser.parse(channel, formats),
            [] => Err(vec![ParseError::Invalid {
                path: Some(vec!["type".into()]),
                message: "unknown channel type".to_string(),
            }]),
            _ => Err(vec![ParseError::Invalid {
                path: None,
                message: format!("{} channel parsers matched", matching.len()),
            }]),
        }
    }
}

/// Prepends `prefix` to each error's path, so errors from a nested parser say where they came from.
fn prefix_paths(errors: Vec<ParseError>, prefix: &[&str]) -> Vec<ParseError> {
    errors
        .into_iter()
        .map(|ParseError::Invalid { path, message }| {
            let mut full: Vec<String> = prefix.iter().map(|s| s.to_string()).collect();
            full.extend(path.unwrap_or_default());
            ParseError::Invalid {
                path: Some(full),
                message,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy_errors(channels: serde_json::Value) -> Vec<ParseError> {
        let spec =
            spec::from_value(serde_json::json!({ "effects": {}, "channels": channels })).unwrap();
        match Dialect::primitive().parse_proxy(spec) {
            Ok(_) => panic!("expected parse errors"),
            Err(errors) => errors,
        }
    }

    fn paths(errors: &[ParseError]) -> Vec<String> {
        errors
            .iter()
            .map(|ParseError::Invalid { path, .. }| path.clone().unwrap_or_default().join("."))
            .collect()
    }

    #[test]
    fn format_errors_name_the_channel() {
        let channel = serde_json::json!({ "type": "stream", "command": "cat" });
        for format in [
            serde_json::json!({ "type": "template" }),
            serde_json::json!({ "type": "template", "template": 1 }),
            serde_json::json!({ "type": "template", "template": "{{#if}}" }),
            serde_json::json!({ "type": "nope" }),
        ] {
            let mut channel = channel.clone();
            channel["format"] = format;
            let errors = proxy_errors(serde_json::json!({ "logger": channel }));
            assert_eq!(paths(&errors).len(), 1);
            assert!(
                paths(&errors)[0].starts_with("channels.logger.format"),
                "{:?}",
                paths(&errors)
            );
        }
    }

    #[test]
    fn exec_command_is_rejected_with_a_pointer_to_format() {
        let errors = proxy_errors(serde_json::json!({
            "logger": { "type": "exec", "command": "echo hi" }
        }));
        assert_eq!(paths(&errors), vec!["channels.logger.command"]);
        assert!(errors[0].to_string().contains("format"));
    }

    #[test]
    fn nested_target_is_rejected_with_a_pointer_to_the_flat_form() {
        let errors = proxy_errors(serde_json::json!({
            "logger": { "target": { "type": "stream", "command": "cat" } }
        }));
        assert_eq!(paths(&errors), vec!["channels.logger.target"]);
        assert!(errors[0].to_string().contains("type"));
    }

    #[test]
    fn unknown_channel_type_names_the_type_field() {
        let errors = proxy_errors(serde_json::json!({ "logger": { "type": "nope" } }));
        assert_eq!(paths(&errors), vec!["channels.logger.type"]);
    }

    #[test]
    fn spec_seed_is_applied_to_the_simulation_builder() {
        let value = serde_json::json!({ "seed": 42, "effects": {} });
        let builder = Dialect::primitive().parse_simulation_json(value).unwrap();
        assert_eq!(builder.seed, 42);
    }

    #[test]
    fn missing_spec_seed_defaults_to_one() {
        let value = serde_json::json!({ "effects": {} });
        let builder = Dialect::primitive().parse_simulation_json(value).unwrap();
        assert_eq!(builder.seed, 1);
    }
}
