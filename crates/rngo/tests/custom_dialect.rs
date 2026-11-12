use rngo::channel::{Channel, ChannelBuilder, ChannelOutput, ChannelParser, OutputSender};
use rngo::format::{Format, FormatParseContext, FormatParser};
use rngo::{
    BuildError, ChannelError, Dialect, FormatError, Input, Level, ParseError, SimpleEventRunLog,
    parsers, spec,
};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

type Sink = Rc<RefCell<Vec<String>>>;

#[derive(Debug)]
struct Upper;

impl Format for Upper {
    fn format(&self, input: &Input) -> Result<String, FormatError> {
        Ok(input.data.as_str().unwrap_or_default().to_uppercase())
    }
}

struct UpperParser;

impl FormatParser for UpperParser {
    fn key(&self) -> &str {
        "upper"
    }

    fn parse(
        &self,
        _format: &spec::Format,
        _spec: &spec::Spec,
    ) -> Result<Box<dyn Format>, Vec<ParseError>> {
        Ok(Box::new(Upper))
    }
}

#[derive(Debug)]
struct Memory {
    sink: Sink,
    format: Option<Rc<dyn Format>>,
}

impl Channel for Memory {
    fn send(&mut self, input: &Input) -> Result<Vec<ChannelOutput>, ChannelError> {
        let line = match &self.format {
            Some(format) => format.format(input).unwrap(),
            None => input.data.to_string(),
        };
        self.sink.borrow_mut().push(line);
        Ok(vec![ChannelOutput::new(Level::Info, "stored")])
    }
}

struct MemoryBuilder {
    sink: Sink,
    format: Option<Rc<dyn Format>>,
}

impl ChannelBuilder for MemoryBuilder {
    fn build(
        &self,
        _channel_key: &str,
        _outputs: OutputSender,
    ) -> Result<Box<dyn Channel>, Vec<BuildError>> {
        Ok(Box::new(Memory {
            sink: self.sink.clone(),
            format: self.format.clone(),
        }))
    }
}

struct MemoryParser {
    sink: Sink,
}

impl ChannelParser for MemoryParser {
    fn key(&self) -> &str {
        "memory"
    }

    fn parse(
        &self,
        channel: &spec::Channel,
        formats: &FormatParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, Vec<ParseError>> {
        let format = match channel.fields.get("format") {
            Some(value) => formats.parse(&["format"], value)?,
            None => None,
        };
        Ok(Box::new(MemoryBuilder {
            sink: self.sink.clone(),
            format,
        }))
    }
}

fn dialect(sink: &Sink) -> Dialect {
    Dialect::new(
        vec![Box::new(parsers::constant())],
        vec![Box::new(UpperParser)],
        vec![Box::new(MemoryParser { sink: sink.clone() })],
        vec![],
    )
}

fn spec_with(generator: Value) -> spec::Spec {
    spec::from_value(json!({
        "effects": {
            "greeting": {
                "trigger": "hz(1, hour)",
                "limit": 2,
                "channel": "mem",
                "generator": generator
            }
        },
        "channels": {
            "mem": { "type": "memory", "format": { "type": "upper" } }
        }
    }))
    .unwrap()
}

#[test]
fn a_custom_dialect_runs_custom_channels_and_formats() {
    let sink = Sink::default();
    let dialect = dialect(&sink);
    let spec = spec_with(json!({ "type": "constant", "value": "hi" }));
    let log = SimpleEventRunLog::new();

    let simulation = dialect
        .parse_simulation(spec.clone())
        .unwrap()
        .run_log_reader(log.clone())
        .build()
        .unwrap();
    let mut proxy = dialect
        .parse_proxy(spec)
        .unwrap()
        .effect_channels(simulation.effect_channels())
        .run_log_writer(log.clone())
        .build()
        .unwrap();
    for step in simulation {
        if let Some(input) = step.unwrap().into_input() {
            proxy.send(&input).unwrap();
        }
    }
    proxy.finish().unwrap();

    assert_eq!(*sink.borrow(), ["HI", "HI"]);
}

#[test]
fn a_custom_dialect_leaves_out_the_built_ins_it_does_not_list() {
    let sink = Sink::default();
    let spec = spec_with(json!({ "type": "number" }));

    assert!(dialect(&sink).parse_simulation(spec).is_err());
}
