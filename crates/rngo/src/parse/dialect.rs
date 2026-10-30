use super::format::{FormatParseContext, FormatParser};
use super::generator::{GeneratorParseVisitor, GeneratorParser};
use super::signal::SignalParser;
use crate::ParseErrorKind;
use crate::audit::signal::Signal;
use crate::audit::{Audit, AuditBuilder, signal};
use crate::effect::generator;
use crate::effect::generator::custom::CustomParser;
use crate::effect::scenario::Scenario;
use crate::effect::simulation::{Simulation, SimulationBuilder};
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
        let mut scenario_builder = Scenario::builder();
        let simulation_moment_parser = Moment::parser();

        if let Some(seed) = spec.seed {
            scenario_builder.set_seed(seed);
        }

        if let Some(start) = &spec.start {
            match simulation_moment_parser.parse("start", start) {
                Ok(timestamp) => {
                    scenario_builder.set_start(timestamp);
                }
                Err(mut e) => errors.append(&mut e),
            };
        };

        if let Some(end) = &spec.end {
            match simulation_moment_parser.parse("end", end) {
                Ok(timestamp) => {
                    scenario_builder.set_end(timestamp);
                }
                Err(mut e) => errors.append(&mut e),
            };
        };

        if spec.scenarios.is_some() {
            errors
                .push(ParseError::invalid("the default phase has no scenarios").at(["scenarios"]));
        }

        let mut channel_phases: HashMap<&str, &str> = HashMap::new();
        let mut effect_phases: HashMap<&str, &str> = HashMap::new();

        for (name, phase) in &spec.phases {
            scenario_builder.with_phase(name, |mut builder| {
                for (field, expr) in [("start", &phase.start), ("end", &phase.end)] {
                    let Some(expr) = expr else { continue };
                    match simulation_moment_parser.parse(field, expr) {
                        Ok(moment) if field == "start" => {
                            builder.set_start(moment);
                        }
                        Ok(moment) => {
                            builder.set_end(moment);
                        }
                        Err(e) => {
                            errors.extend(e.into_iter().map(|e| e.prefixed(&["phases", name])))
                        }
                    }
                }
                builder
            });

            for channel in &phase.channels {
                if !spec.channels.contains_key(channel) {
                    errors.push(
                        ParseError::new(ParseErrorKind::UnknownChannel(channel.clone()))
                            .at(["phases", name, "channels"]),
                    );
                } else if let Some(other) = channel_phases.insert(channel, name) {
                    errors.push(
                        ParseError::invalid(format!(
                            "channel \"{channel}\" is already claimed by phase \"{other}\""
                        ))
                        .at(["phases", name, "channels"]),
                    );
                }
            }

            for effect in &phase.effects {
                if !spec.effects.contains_key(effect) {
                    errors.push(
                        ParseError::invalid(format!("unknown effect \"{effect}\""))
                            .at(["phases", name, "effects"]),
                    );
                } else if let Some(other) = effect_phases.insert(effect, name) {
                    errors.push(
                        ParseError::invalid(format!(
                            "effect \"{effect}\" is already in phase \"{other}\""
                        ))
                        .at(["phases", name, "effects"]),
                    );
                }
            }
        }

        for name in spec.generators.keys() {
            if self.generator_parsers.iter().any(|p| p.key() == name) {
                errors.push(ParseError::invalid(format!(
                        "\"{name}\" is a primitive generator type and cannot be used as a custom generator name"
                    )).at(["generators", name.as_str()]));
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

        let scenario = match scenario_builder.build() {
            Ok(scenario) => Some(scenario),
            Err(e) => {
                errors.extend(e.iter().map(|e| {
                    let message = e.message();
                    if message.starts_with("phase ") {
                        ParseError::invalid(message).at(["phases"])
                    } else {
                        ParseError::invalid(message).at(["end"])
                    }
                }));
                None
            }
        };

        for (key, effect) in &spec.effects {
            let Some(scenario) = &scenario else { break };
            let phase = effect_phases.get(key.as_str()).copied().or_else(|| {
                let channel = effect.channel.as_deref()?;
                channel_phases.get(channel).copied()
            });
            let resolved = scenario.phase(phase).unwrap();
            let (start, end) = (Moment::from(resolved.start), Moment::from(resolved.end));
            let effect_moment_parser = Moment::parser().simulation(&start, &end);

            let visitor = GeneratorParseVisitor::new(
                self.generator_parsers.clone(),
                Rc::clone(&custom_generators),
                effect.generator.clone(),
                vec![],
                vec!["effects".into(), key.clone(), "generator".into()],
            );

            let generator_builder = match visitor.parse() {
                Ok(generator_builder) => generator_builder,
                Err(mut e) => {
                    errors.append(&mut e);
                    continue;
                }
            };

            simulation_builder.with_effect(key, |mut effect_builder| {
                effect_builder.set_generator(generator_builder);
                effect_builder
                    .set_scenario(scenario)
                    .set_phase(resolved.clone());

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
                        spec::Trigger::Clock { rate } => {
                            effect_builder.set_trigger_expression(rate)
                        }
                        spec::Trigger::Effect { key } => effect_builder.set_trigger_effect(key),
                    };
                }

                effect_builder
            });
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
                errors.push(
                    ParseError::new(ParseErrorKind::UnknownChannel(channel_key.clone())).at([
                        "effects",
                        effect_key.as_str(),
                        "channel",
                    ]),
                );
            }
        }

        let formats = FormatParseContext::new(&self.format_parsers, &spec);

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
                        proxy_builder.set_channel(key.clone(), channel_builder);
                    }
                    Err(e) => errors.extend(e.into_iter().map(|e| e.prefixed(&["channels", key]))),
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
            [] => Err(vec![ParseError::new(ParseErrorKind::UnknownType {
                kind: "signal",
            })]),
            _ => Err(vec![ParseError::new(ParseErrorKind::AmbiguousType {
                kind: "signal",
                count: matching.len(),
            })]),
        }
    }

    fn parse_channel(
        &self,
        channel: &spec::Channel,
        formats: &FormatParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, Vec<ParseError>> {
        let matching: Vec<_> = self
            .channel_parsers
            .iter()
            .filter(|p| channel.ctype.as_deref() == Some(p.key()))
            .collect();

        match matching.as_slice() {
            [parser] => parser.parse(channel, formats),
            [] => Err(vec![
                ParseError::new(ParseErrorKind::UnknownType { kind: "channel" }).at(["type"]),
            ]),
            _ => Err(vec![ParseError::new(ParseErrorKind::AmbiguousType {
                kind: "channel",
                count: matching.len(),
            })]),
        }
    }
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
        assert_eq!(paths(&errors), vec!["channels.logger.type"]);
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
            .parse_simulation_json(value)
            .unwrap()
            .standalone()
            .unwrap()
            .flatten()
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
