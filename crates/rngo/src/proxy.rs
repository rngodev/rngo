pub mod channel;
pub mod format;
pub mod output;

use crate::{BuildError, Input, Level, RunLogWriter, SimpleEventRunLog, TargetOutput};
use channel::target::Stdout;
use channel::{Channel, ChannelBuilder};
use chrono::{DateTime, FixedOffset};
use output::Output;
use serde_json::Value;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};

/// Decides when the proxy may send each input.
pub trait Pacer {
    /// Blocks until an input with `timestamp` may be sent; returns `false` to stop the proxy instead.
    fn wait_until(&mut self, timestamp: DateTime<FixedOffset>) -> bool;
}

pub struct Proxy {
    run_log_writer: Rc<dyn RunLogWriter>,
    channels: HashMap<String, Channel>,
    effect_channels: HashMap<String, String>,
    output_rx: Receiver<Output>,
    pacer: Option<Box<dyn Pacer>>,
    stopped: bool,
}

impl Proxy {
    pub fn builder() -> ProxyBuilder {
        ProxyBuilder::new()
    }

    /// Sends `input` to its effect's channel, first waiting on the pacer if there is one.
    pub fn send(&mut self, input: &Input) -> Result<(), Box<dyn std::error::Error>> {
        if self.stopped {
            return Ok(());
        }

        if let Some(pacer) = self.pacer.as_mut()
            && !pacer.wait_until(input.timestamp)
        {
            self.stopped = true;
            return Ok(());
        }

        let channel_key = match self.effect_channels.get(&input.effect) {
            Some(k) => k,
            None => return Ok(()),
        };

        if let Some(channel) = self.channels.get_mut(channel_key) {
            let data = match channel.format.as_ref().map(|f| f.format(input)) {
                Some(Ok(data)) => Value::String(data),
                Some(Err(message)) => {
                    self.run_log_writer.push_output(
                        TargetOutput::new(Level::Error, format!("format failed: {message}"))
                            .into_output(&channel.key, Some(input.id)),
                    );
                    self.drain_outputs();
                    return Ok(());
                }
                None => input.data.clone(),
            };

            for output in channel.target.send(data)? {
                self.run_log_writer
                    .push_output(output.into_output(&channel.key, Some(input.id)));
            }
        };

        self.drain_outputs();
        Ok(())
    }

    /// Whether the pacer has stopped the proxy; once stopped, `send` ignores further inputs.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    pub fn finish(&mut self) {
        for channel in self.channels.values_mut() {
            channel.target.finish();
        }
        self.drain_outputs();
    }

    fn drain_outputs(&mut self) {
        while let Ok(output) = self.output_rx.try_recv() {
            self.run_log_writer.push_output(output);
        }
    }
}

pub struct ProxyBuilder {
    run_log_writer: Option<Rc<dyn RunLogWriter>>,
    channel_builders: Vec<ChannelBuilder>,
    stdout: bool,
    pacer: Option<Box<dyn Pacer>>,
}

impl ProxyBuilder {
    pub fn new() -> Self {
        Self {
            run_log_writer: None,
            channel_builders: vec![],
            stdout: false,
            pacer: None,
        }
    }

    pub fn run_log_writer<T: RunLogWriter + 'static>(mut self, writer: Rc<T>) -> Self {
        self.run_log_writer = Some(writer as Rc<dyn RunLogWriter>);
        self
    }

    /// Paces sends with `pacer`; without one, inputs are sent as soon as they arrive.
    pub fn pacer(mut self, pacer: impl Pacer + 'static) -> Self {
        self.set_pacer(pacer);
        self
    }

    pub fn set_pacer(&mut self, pacer: impl Pacer + 'static) -> &mut Self {
        self.pacer = Some(Box::new(pacer));
        self
    }

    pub fn stdout(mut self, value: bool) -> Self {
        self.set_stdout(value);
        self
    }

    pub fn set_stdout(&mut self, value: bool) -> &mut Self {
        self.stdout = value;
        self
    }

    pub fn set_channel(&mut self, channel: ChannelBuilder) {
        self.channel_builders.push(channel)
    }

    pub fn with_channel(
        &mut self,
        key: &str,
        f: impl FnOnce(ChannelBuilder) -> ChannelBuilder,
    ) -> &mut Self {
        let builder = Channel::builder(key.into());
        let builder = f(builder);
        self.channel_builders.push(builder);
        self
    }

    pub fn build(self) -> Result<Proxy, Vec<BuildError>> {
        let mut errors = vec![];
        let mut channels = HashMap::new();
        let (output_tx, output_rx) = mpsc::channel::<Output>();

        let run_log_writer = self
            .run_log_writer
            .unwrap_or_else(|| SimpleEventRunLog::new());

        for mut channel_builder in self.channel_builders {
            if self.stdout {
                channel_builder.set_target(Box::new(Stdout::builder()));
            }
            channel_builder.set_output_tx(output_tx.clone());

            match channel_builder.build() {
                Ok(channel) => {
                    channels.insert(channel.key.clone(), channel);
                }
                Err(mut e) => errors.append(&mut e),
            };
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        let effect_channels = channels
            .values()
            .flat_map(|channel| {
                channel
                    .effects
                    .iter()
                    .map(move |effect_key| (effect_key.clone(), channel.key.clone()))
            })
            .collect();

        Ok(Proxy {
            run_log_writer,
            channels,
            effect_channels,
            output_rx,
            pacer: self.pacer,
            stopped: false,
        })
    }
}

impl Default for ProxyBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OutputSender;
    use crate::log::Metadata;
    use crate::proxy::channel::{ChannelTarget, ChannelTargetBuilder};
    use crate::proxy::format::Format;
    use chrono::Utc;
    use serde_json::json;
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    struct RecordingLog {
        outputs: RefCell<Vec<Output>>,
    }

    impl RunLogWriter for RecordingLog {
        fn push_input(&self, _input: Input) {}
        fn push_output(&self, output: Output) {
            self.outputs.borrow_mut().push(output);
        }
        fn push_metadata(&self, _metadata: Metadata) {}
    }

    #[derive(Debug)]
    struct FailOnEvenIds;

    impl Format for FailOnEvenIds {
        fn format(&self, event: &Input) -> Result<String, String> {
            if event.id.is_multiple_of(2) {
                Err("boom".into())
            } else {
                Ok(event.id.to_string())
            }
        }
    }

    #[derive(Debug)]
    struct RecordingTarget(Rc<RefCell<Vec<Value>>>);

    impl ChannelTarget for RecordingTarget {
        fn send(&mut self, data: Value) -> Result<Vec<TargetOutput>, Box<dyn std::error::Error>> {
            self.0.borrow_mut().push(data);
            Ok(vec![])
        }
    }

    struct RecordingTargetBuilder(Rc<RefCell<Vec<Value>>>);

    impl ChannelTargetBuilder for RecordingTargetBuilder {
        fn build(
            &self,
            _channel_key: &str,
            _outputs: OutputSender,
        ) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>> {
            Ok(Box::new(RecordingTarget(self.0.clone())))
        }
    }

    fn input(id: u64) -> Input {
        Input {
            id,
            effect: "ping".into(),
            offset: 0,
            timestamp: Utc::now().fixed_offset(),
            data: json!(null),
            metadata: vec![],
        }
    }

    #[test]
    fn format_failure_records_an_error_output_and_keeps_going() {
        let log = Rc::new(RecordingLog::default());
        let sent = Rc::new(RefCell::new(vec![]));
        let mut builder = Proxy::builder().run_log_writer(log.clone());
        builder.with_channel("logger", |c| {
            c.format(FailOnEvenIds)
                .target(RecordingTargetBuilder(sent.clone()))
                .effects(vec!["ping".into()])
        });
        let mut proxy = builder.build().unwrap();

        for id in 1..=3 {
            proxy.send(&input(id)).unwrap();
        }

        assert_eq!(*sent.borrow(), vec![json!("1"), json!("3")]);
        let outputs = log.outputs.borrow();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].input_id, Some(2));
        assert_eq!(outputs[0].channel, "logger");
        assert!(matches!(outputs[0].level, Level::Error));
        assert!(outputs[0].data.contains("boom"));
    }

    #[test]
    fn unformatted_channels_send_the_input_data() {
        let sent = Rc::new(RefCell::new(vec![]));
        let mut builder = Proxy::builder();
        builder.with_channel("logger", |c| {
            c.target(RecordingTargetBuilder(sent.clone()))
                .effects(vec!["ping".into()])
        });
        let mut proxy = builder.build().unwrap();

        let mut ping = input(1);
        ping.data = json!({ "a": 1 });
        proxy.send(&ping).unwrap();

        assert_eq!(*sent.borrow(), vec![json!({ "a": 1 })]);
    }

    #[test]
    fn target_outputs_are_tied_to_the_input() {
        let log = Rc::new(RecordingLog::default());
        let mut builder = Proxy::builder().run_log_writer(log.clone());
        builder.with_channel("logger", |c| {
            c.target(channel::target::Exec::builder())
                .effects(vec!["ping".into()])
        });
        let mut proxy = builder.build().unwrap();

        let mut ping = input(5);
        ping.data = json!("echo hi");
        proxy.send(&ping).unwrap();

        let outputs = log.outputs.borrow();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].input_id, Some(5));
        assert_eq!(outputs[0].data, "hi");
    }

    struct RecordingPacer {
        calls: Rc<RefCell<Vec<DateTime<FixedOffset>>>>,
        allow: usize,
    }

    impl Pacer for RecordingPacer {
        fn wait_until(&mut self, timestamp: DateTime<FixedOffset>) -> bool {
            self.calls.borrow_mut().push(timestamp);
            self.calls.borrow().len() <= self.allow
        }
    }

    #[test]
    fn pacer_is_consulted_before_each_send() {
        let sent = Rc::new(RefCell::new(vec![]));
        let calls = Rc::new(RefCell::new(vec![]));
        let mut builder = Proxy::builder().pacer(RecordingPacer {
            calls: calls.clone(),
            allow: usize::MAX,
        });
        builder.with_channel("logger", |c| {
            c.target(RecordingTargetBuilder(sent.clone()))
                .effects(vec!["ping".into()])
        });
        let mut proxy = builder.build().unwrap();

        let inputs: Vec<Input> = (1..=3).map(input).collect();
        for input in &inputs {
            proxy.send(input).unwrap();
        }

        let timestamps: Vec<_> = inputs.iter().map(|i| i.timestamp).collect();
        assert_eq!(*calls.borrow(), timestamps);
        assert_eq!(sent.borrow().len(), 3);
        assert!(!proxy.stopped());
    }

    #[test]
    fn stopping_pacer_stops_the_proxy() {
        let sent = Rc::new(RefCell::new(vec![]));
        let calls = Rc::new(RefCell::new(vec![]));
        let mut builder = Proxy::builder().pacer(RecordingPacer {
            calls: calls.clone(),
            allow: 1,
        });
        builder.with_channel("logger", |c| {
            c.target(RecordingTargetBuilder(sent.clone()))
                .effects(vec!["ping".into()])
        });
        let mut proxy = builder.build().unwrap();

        for id in 1..=3 {
            let mut ping = input(id);
            ping.data = json!(id);
            proxy.send(&ping).unwrap();
        }

        assert_eq!(*sent.borrow(), vec![json!(1)]);
        assert_eq!(calls.borrow().len(), 2);
        assert!(proxy.stopped());
    }
}
