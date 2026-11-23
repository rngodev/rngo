use rngo::channel::{
    Channel, ChannelBuildContext, ChannelBuilder, ChannelOutput, ChannelParseContext,
    ChannelParser, SendContext,
};
use rngo::format::{Format, FormatBuilder, FormatContext, FormatParseContext, FormatParser};
use rngo::{
    BuildErrors, ChannelError, Dialect, FormatError, Input, Level, ParseErrors, SimpleEventRunLog,
    parse, spec,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

type Sink = Arc<Mutex<Vec<String>>>;

#[derive(Debug)]
struct Upper;

impl Format for Upper {
    fn format(&self, input: &Input, _context: &FormatContext) -> Result<String, FormatError> {
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
        _context: &FormatParseContext,
    ) -> Result<Box<dyn FormatBuilder>, ParseErrors> {
        Ok(Box::new(Upper))
    }
}

#[derive(Debug)]
struct Memory {
    sink: Sink,
    format: Option<Box<dyn Format>>,
}

impl Channel for Memory {
    fn send(
        &mut self,
        input: &Input,
        context: &SendContext,
    ) -> Result<Vec<ChannelOutput>, ChannelError> {
        let line = match &self.format {
            Some(format) => format.format(input, context.format_context()).unwrap(),
            None => input.data.to_string(),
        };
        self.sink.lock().unwrap().push(line);
        Ok(vec![ChannelOutput::new(Level::Info, "stored")])
    }
}

struct MemoryBuilder {
    sink: Sink,
    format: Option<Box<dyn FormatBuilder>>,
}

impl ChannelBuilder for MemoryBuilder {
    fn build(
        self: Box<Self>,
        context: ChannelBuildContext,
    ) -> Result<Box<dyn Channel>, BuildErrors> {
        Ok(Box::new(Memory {
            sink: self.sink,
            format: self
                .format
                .map(|format| format.build(context.format_context()))
                .transpose()?,
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
        context: &ChannelParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, ParseErrors> {
        let format = match channel.fields.get("format") {
            Some(value) => context.parse_format(&["format"], value)?,
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
        vec![Box::new(parse::constant())],
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
    let log = std::sync::Arc::new(SimpleEventRunLog::new());

    let simulation = dialect
        .parse_simulation(&spec)
        .unwrap()
        .run_log_reader(log.clone())
        .build()
        .unwrap();
    let mut proxy = dialect
        .parse_proxy(&spec)
        .unwrap()
        .run_log_writer(log.clone())
        .build()
        .unwrap();
    for step in simulation {
        if let Some(input) = step.unwrap().into_input() {
            proxy.send(&input).unwrap();
        }
    }
    proxy.finish().unwrap();

    assert_eq!(*sink.lock().unwrap(), ["HI", "HI"]);
}

#[test]
fn a_custom_dialect_leaves_out_the_built_ins_it_does_not_list() {
    let sink = Sink::default();
    let spec = spec_with(json!({ "type": "number" }));

    assert!(dialect(&sink).parse_simulation(&spec).is_err());
}
