use crate::ParseErrorKind;
use crate::audit::signal::{SignalBuilder, SignalParser};
use crate::audit::{Audit, AuditBuilder};
use crate::effect::generator::custom::CustomParser;
use crate::effect::generator::{GeneratorParseVisitor, GeneratorParser};
use crate::effect::simulation::{Simulation, SimulationBuilder};
use crate::error::{ParseError, ParseErrors};
use crate::moment::Moment;
use crate::proxy::channel::{ChannelBuilder, ChannelParser};
use crate::proxy::format::{FormatParseContext, FormatParser};
use crate::proxy::{Proxy, ProxyBuilder};
use crate::spec::{self, Spec};
use std::sync::Arc;

pub struct Dialect {
    generator_parsers: Arc<Vec<Box<dyn GeneratorParser>>>,
    format_parsers: Arc<Vec<Box<dyn FormatParser>>>,
    channel_parsers: Arc<Vec<Box<dyn ChannelParser>>>,
    signal_parsers: Arc<Vec<Box<dyn SignalParser>>>,
}

impl Dialect {
    /// A dialect with exactly these parsers; see [`crate::parse`] for the built-in ones.
    pub fn new(
        generator_parsers: Vec<Box<dyn GeneratorParser>>,
        format_parsers: Vec<Box<dyn FormatParser>>,
        channel_parsers: Vec<Box<dyn ChannelParser>>,
        signal_parsers: Vec<Box<dyn SignalParser>>,
    ) -> Self {
        Dialect {
            generator_parsers: Arc::new(generator_parsers),
            format_parsers: Arc::new(format_parsers),
            channel_parsers: Arc::new(channel_parsers),
            signal_parsers: Arc::new(signal_parsers),
        }
    }

    /// Adds a parser for a generator `type`, alongside the dialect's existing ones.
    pub fn with_generator_parser(mut self, parser: impl GeneratorParser + 'static) -> Self {
        Arc::get_mut(&mut self.generator_parsers)
            .expect("generator parsers are only shared during a parse")
            .push(Box::new(parser));
        self
    }

    /// Adds a parser for a format `type`, alongside the dialect's existing ones.
    pub fn with_format_parser(mut self, parser: impl FormatParser + 'static) -> Self {
        Arc::get_mut(&mut self.format_parsers)
            .expect("format parsers are only shared during a parse")
            .push(Box::new(parser));
        self
    }

    /// Adds a parser for a channel `type`, alongside the dialect's existing ones.
    pub fn with_channel_parser(mut self, parser: impl ChannelParser + 'static) -> Self {
        Arc::get_mut(&mut self.channel_parsers)
            .expect("channel parsers are only shared during a parse")
            .push(Box::new(parser));
        self
    }

    /// Adds a parser for a signal `type`, alongside the dialect's existing ones.
    pub fn with_signal_parser(mut self, parser: impl SignalParser + 'static) -> Self {
        Arc::get_mut(&mut self.signal_parsers)
            .expect("signal parsers are only shared during a parse")
            .push(Box::new(parser));
        self
    }

    /// The built-in generator, format, channel and signal parsers.
    pub fn primitive() -> Self {
        use crate::parse::*;
        Dialect::new(
            vec![
                Box::new(array()),
                Box::new(constant()),
                Box::new(context()),
                Box::new(function()),
                Box::new(number()),
                Box::new(object()),
                Box::new(reference()),
                Box::new(select()),
                Box::new(string()),
            ],
            vec![Box::new(sql_format()), Box::new(template_format())],
            vec![Box::new(exec()), Box::new(stdout()), Box::new(stream())],
            vec![Box::new(sql_signal())],
        )
    }

    /// Parses the spec's seed, time window, prototypes and effects into a simulation builder.
    pub fn parse_simulation(&self, spec: &Spec) -> Result<SimulationBuilder, ParseErrors> {
        let mut errors = vec![];
        let mut simulation_builder = Simulation::builder();
        let simulation_moment_parser = Moment::parser();

        if let Some(seed) = spec.seed {
            simulation_builder = simulation_builder.seed(seed);
        }

        if let Some(start) = &spec.start {
            match simulation_moment_parser.parse("start", start) {
                Ok(timestamp) => {
                    simulation_builder = simulation_builder.start(timestamp);
                }
                Err(e) => errors.extend(e),
            };
        };

        if let Some(end) = &spec.end {
            match simulation_moment_parser.parse("end", end) {
                Ok(timestamp) => {
                    simulation_builder = simulation_builder.end(timestamp);
                }
                Err(e) => errors.extend(e),
            };
        };

        if spec.scenarios.is_some() {
            errors.push(
                ParseError::invalid("the default prototype has no scenarios").at(["scenarios"]),
            );
        }

        for (name, prototype) in &spec.prototypes {
            simulation_builder = simulation_builder.prototype(name, |mut prototype_builder| {
                if let Some(start) = &prototype.start {
                    prototype_builder = prototype_builder.start(start.as_str());
                }

                if let Some(end) = &prototype.end {
                    prototype_builder = prototype_builder.end(end.as_str());
                }

                if let Some(limit) = prototype.limit {
                    prototype_builder = prototype_builder.limit(limit.get());
                }

                if let Some(channel) = &prototype.channel {
                    prototype_builder = prototype_builder.channel(channel.clone());
                }

                prototype_builder
            });
        }

        for name in spec.generators.keys() {
            if self.generator_parsers.iter().any(|p| p.key() == name) {
                errors.push(ParseError::invalid(format!(
                        "\"{name}\" is a primitive generator type and cannot be used as a custom generator name"
                    )).at(["generators", name.as_str()]));
            }
        }

        let custom_generators: Arc<Vec<CustomParser>> = Arc::new(
            spec.generators
                .iter()
                .map(|(name, generator_type)| {
                    CustomParser::new(name.clone(), generator_type.clone())
                })
                .collect(),
        );

        for (key, effect) in &spec.effects {
            if let Some(prototype) = &effect.prototype
                && !spec.prototypes.contains_key(prototype)
            {
                errors.push(
                    ParseError::invalid(format!(
                        "effect \"{key}\" names unknown prototype \"{prototype}\""
                    ))
                    .at(["effects", key.as_str(), "prototype"]),
                );
                continue;
            }

            let visitor = GeneratorParseVisitor::new(
                self.generator_parsers.clone(),
                Arc::clone(&custom_generators),
                effect.generator.clone(),
                vec![],
                vec!["effects".into(), key.clone(), "generator".into()],
            );

            let generator_builder = match visitor.parse() {
                Ok(generator_builder) => generator_builder,
                Err(e) => {
                    errors.extend(e);
                    continue;
                }
            };

            simulation_builder = simulation_builder.effect(key, |mut effect_builder| {
                effect_builder = effect_builder.generator(generator_builder);
                if let Some(prototype) = &effect.prototype {
                    effect_builder = effect_builder.prototype_key(prototype.clone());
                }

                if let Some(start) = &effect.start {
                    effect_builder = effect_builder.start(start.as_str());
                }

                if let Some(end) = &effect.end {
                    effect_builder = effect_builder.end(end.as_str());
                }

                if let Some(limit) = effect.limit {
                    effect_builder = effect_builder.limit(limit.get());
                }

                if let Some(channel) = &effect.channel {
                    effect_builder = effect_builder.channel(channel.clone());
                }

                if let Some(trigger_union) = &effect.trigger {
                    let trigger = match trigger_union {
                        spec::TriggerUnion::Shorthand(rate) => {
                            spec::Trigger::Clock { rate: rate.clone() }
                        }
                        spec::TriggerUnion::Full(trigger) => trigger.clone(),
                    };

                    effect_builder = match trigger {
                        spec::Trigger::Clock { rate } => effect_builder.trigger_expression(rate),
                        spec::Trigger::Effect { key } => effect_builder.trigger_effect(key),
                    };
                }

                effect_builder
            });
        }

        if !errors.is_empty() {
            Err(errors.into())
        } else {
            Ok(simulation_builder)
        }
    }

    /// Parses the spec's channels, and checks the channels its effects and prototypes name.
    pub fn parse_proxy(&self, spec: &Spec) -> Result<ProxyBuilder, ParseErrors> {
        let mut errors = vec![];
        let mut proxy_builder = Proxy::builder();

        for (name, prototype) in &spec.prototypes {
            if let Some(channel_key) = &prototype.channel
                && !spec.channels.contains_key(channel_key)
            {
                errors.push(
                    ParseError::new(ParseErrorKind::UnknownChannel(channel_key.clone())).at([
                        "prototypes",
                        name.as_str(),
                        "channel",
                    ]),
                );
            }
        }

        for (effect_key, effect) in &spec.effects {
            if let Some(channel_key) = &effect.channel
                && !spec.channels.contains_key(channel_key)
            {
                errors.push(
                    ParseError::new(ParseErrorKind::UnknownChannel(channel_key.clone())).at([
                        "effects",
                        effect_key.as_str(),
                        "channel",
                    ]),
                );
            }
        }

        let formats = FormatParseContext::new(&self.format_parsers, spec);

        for (key, channel) in &spec.channels {
            if channel.fields.contains_key("target") {
                errors.push(
                    ParseError::invalid(
                        "channels no longer take a target; set `type` (and e.g. `command`) \
                              directly on the channel",
                    )
                    .at(["channels", key.as_str(), "target"]),
                );
            } else {
                match self.parse_channel(channel, &formats) {
                    Ok(channel_builder) => {
                        proxy_builder = proxy_builder.channel(key.clone(), channel_builder);
                    }
                    Err(e) => errors.extend(e.into_iter().map(|e| e.prefixed(&["channels", key]))),
                }
            }
        }

        if !errors.is_empty() {
            Err(errors.into())
        } else {
            Ok(proxy_builder)
        }
    }

    /// Parses the spec's signals into an audit builder.
    pub fn parse_audit(&self, spec: &Spec) -> Result<AuditBuilder, ParseErrors> {
        let mut errors = vec![];
        let mut audit_builder = Audit::builder();

        for (key, signal) in &spec.signals {
            match self.parse_signal(signal) {
                Ok(built) => {
                    audit_builder = audit_builder.signal(key.clone(), built);
                }
                Err(e) => errors.extend(e.prefixed(&["signals", key])),
            }
        }

        if !errors.is_empty() {
            Err(errors.into())
        } else {
            Ok(audit_builder)
        }
    }

    fn parse_signal(&self, signal: &spec::Signal) -> Result<Box<dyn SignalBuilder>, ParseErrors> {
        let matching: Vec<_> = self
            .signal_parsers
            .iter()
            .filter(|p| signal.stype.as_deref() == Some(p.key()))
            .collect();

        match matching.as_slice() {
            [parser] => parser.parse(signal),
            [] => Err(ParseError::new(ParseErrorKind::UnknownType {
                kind: "signal",
                found: signal.stype.clone(),
            })
            .at(["type"])
            .into()),
            _ => Err(ParseError::new(ParseErrorKind::AmbiguousType {
                kind: "signal",
                count: matching.len(),
            })
            .at(["type"])
            .into()),
        }
    }

    fn parse_channel(
        &self,
        channel: &spec::Channel,
        formats: &FormatParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, ParseErrors> {
        let matching: Vec<_> = self
            .channel_parsers
            .iter()
            .filter(|p| channel.ctype.as_deref() == Some(p.key()))
            .collect();

        match matching.as_slice() {
            [parser] => parser.parse(channel, formats),
            [] => Err(ParseError::new(ParseErrorKind::UnknownType {
                kind: "channel",
                found: channel.ctype.clone(),
            })
            .at(["type"])
            .into()),
            _ => Err(ParseError::new(ParseErrorKind::AmbiguousType {
                kind: "channel",
                count: matching.len(),
            })
            .at(["type"])
            .into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy_errors(channels: serde_json::Value) -> ParseErrors {
        let spec =
            spec::from_value(serde_json::json!({ "effects": {}, "channels": channels })).unwrap();
        match Dialect::primitive().parse_proxy(&spec) {
            Ok(_) => panic!("expected parse errors"),
            Err(errors) => errors,
        }
    }

    fn paths(errors: &[ParseError]) -> Vec<String> {
        errors.iter().map(|e| e.path().join(".")).collect()
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
        assert_eq!(
            errors.to_string(),
            "failed to parse `channels.logger.type`: `unknown channel type \"nope\"`"
        );
    }

    #[test]
    fn unknown_format_type_names_the_type_field() {
        let errors = proxy_errors(serde_json::json!({
            "logger": { "type": "stdout", "format": { "type": "nope" } }
        }));
        assert_eq!(
            errors.to_string(),
            "failed to parse `channels.logger.format.type`: `unknown format type \"nope\"`"
        );
    }

    fn audit_errors(signals: serde_json::Value) -> ParseErrors {
        let spec =
            spec::from_value(serde_json::json!({ "effects": {}, "signals": signals })).unwrap();
        match Dialect::primitive().parse_audit(&spec) {
            Ok(_) => panic!("expected parse errors"),
            Err(errors) => errors,
        }
    }

    #[test]
    fn unknown_signal_type_names_the_type_field() {
        let errors = audit_errors(serde_json::json!({ "s": { "type": "nope" } }));
        assert_eq!(
            errors.to_string(),
            "failed to parse `signals.s.type`: `unknown signal type \"nope\"`"
        );
    }

    #[test]
    fn missing_signal_type_names_the_type_field() {
        let errors = audit_errors(serde_json::json!({ "s": { "query": "select 1" } }));
        assert_eq!(
            errors.to_string(),
            "failed to parse `signals.s.type`: `missing signal type`"
        );
    }

    #[test]
    fn sql_signal_errors_name_the_signal() {
        let errors = audit_errors(serde_json::json!({ "s": { "type": "sql" } }));
        assert_eq!(paths(&errors), vec!["signals.s.query"]);
    }

    fn first_values(seed: Option<u64>) -> Vec<serde_json::Value> {
        let mut value = serde_json::json!({
            "start": "2024-01-01",
            "end": "2024-02-01",
            "effects": {
                "n": {
                    "trigger": "hz(1, day)",
                    "generator": { "type": "number", "minimum": 0, "maximum": 1000000 }
                }
            }
        });
        if let Some(seed) = seed {
            value["seed"] = serde_json::json!(seed);
        }
        Dialect::primitive()
            .parse_simulation(&spec::from_value(value).unwrap())
            .unwrap()
            .standalone()
            .unwrap()
            .filter_map(|step| step.unwrap().into_input())
            .map(|input| input.data)
            .take(5)
            .collect()
    }

    #[test]
    fn spec_seed_is_applied_to_the_simulation() {
        assert_eq!(first_values(Some(42)), first_values(Some(42)));
        assert_ne!(first_values(Some(42)), first_values(Some(1)));
    }

    #[test]
    fn missing_spec_seed_defaults_to_one() {
        assert_eq!(first_values(None), first_values(Some(1)));
    }
}
