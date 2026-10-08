pub mod channel;
pub mod format;
pub mod output;
mod stop;

use crate::{BuildError, Input, Level, Metadata, RunLogWriter, SimpleEventRunLog};
use channel::target::Stdout;
use channel::{Channel, ChannelBuilder};
use chrono::{DateTime, FixedOffset, Utc};
use output::{Output, TargetOutput};
use serde_json::Value;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
pub use stop::StopHandle;

const WAIT_TICK: Duration = Duration::from_secs(1);

type OnWait = Box<dyn FnMut(DateTime<FixedOffset>)>;

pub struct Proxy {
    run_log_writer: Rc<dyn RunLogWriter>,
    channels: HashMap<String, Channel>,
    effect_channels: HashMap<String, String>,
    output_rx: Receiver<Output>,
    realtime: bool,
    on_wait: Option<OnWait>,
    stop: StopHandle,
    started: bool,
    finished: bool,
}

impl Proxy {
    pub fn builder() -> ProxyBuilder {
        ProxyBuilder::new()
    }

    /// Logs `input`, then sends it to its effect's channel. In realtime mode, first waits until its
    /// timestamp. Once stopped, only logs. The first call records the `simulation_start` timing.
    pub fn send(&mut self, input: &Input) -> Result<(), Box<dyn std::error::Error>> {
        if !self.started {
            self.started = true;
            self.record_timing("simulation_start");
        }

        self.run_log_writer.push_input(input.clone());

        if self.stop.is_stopped() || (self.realtime && !self.wait_until(input.timestamp)) {
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

    /// A handle that can stop this proxy from any thread.
    pub fn stop_handle(&self) -> StopHandle {
        self.stop.clone()
    }

    /// Blocks until `timestamp` has passed, calling `on_wait` about once a second meanwhile;
    /// returns `false` if stopped first.
    fn wait_until(&mut self, timestamp: DateTime<FixedOffset>) -> bool {
        loop {
            let Some(remaining) = (timestamp.to_utc() - Utc::now())
                .to_std()
                .ok()
                .filter(|d| !d.is_zero())
            else {
                return true;
            };

            if let Some(on_wait) = self.on_wait.as_mut() {
                on_wait(timestamp);
            }

            if self.stop.wait_timeout(remaining.min(WAIT_TICK)) {
                return false;
            }
        }
    }

    /// Records the `simulation_end` timing (if anything was sent) and drains the channel targets.
    pub fn finish(&mut self) {
        if self.started && !self.finished {
            self.finished = true;
            self.record_timing("simulation_end");
        }

        for channel in self.channels.values_mut() {
            channel.target.finish();
        }
        self.drain_outputs();
    }

    fn record_timing(&self, key: &str) {
        self.run_log_writer.push_metadata(Metadata {
            mtype: "timing".to_string(),
            input_id: None,
            output_id: None,
            data: Some(serde_json::json!({
                "key": key,
                "timestamp": Utc::now().timestamp_millis(),
            })),
            segment: None,
            timestamp: None,
        });
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
    realtime: bool,
    on_wait: Option<OnWait>,
    stop: StopHandle,
}

impl ProxyBuilder {
    pub fn new() -> Self {
        Self {
            run_log_writer: None,
            channel_builders: vec![],
            stdout: false,
            realtime: false,
            on_wait: None,
            stop: StopHandle::new(),
        }
    }

    pub fn run_log_writer<T: RunLogWriter + 'static>(mut self, writer: Rc<T>) -> Self {
        self.run_log_writer = Some(writer as Rc<dyn RunLogWriter>);
        self
    }

    /// When true, holds each input until its timestamp has passed; otherwise sends it right away.
    pub fn realtime(mut self, value: bool) -> Self {
        self.set_realtime(value);
        self
    }

    pub fn set_realtime(&mut self, value: bool) -> &mut Self {
        self.realtime = value;
        self
    }

    /// Called with the awaited timestamp when a realtime wait starts and about once a second
    /// while it lasts.
    pub fn on_wait(mut self, f: impl FnMut(DateTime<FixedOffset>) + 'static) -> Self {
        self.set_on_wait(f);
        self
    }

    pub fn set_on_wait(&mut self, f: impl FnMut(DateTime<FixedOffset>) + 'static) -> &mut Self {
        self.on_wait = Some(Box::new(f));
        self
    }

    /// Uses `handle` to stop the built proxy, so it can be shared before the proxy exists.
    pub fn stop_handle(mut self, handle: StopHandle) -> Self {
        self.set_stop_handle(handle);
        self
    }

    pub fn set_stop_handle(&mut self, handle: StopHandle) -> &mut Self {
        self.stop = handle;
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
            realtime: self.realtime,
            on_wait: self.on_wait,
            stop: self.stop,
            started: false,
            finished: false,
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
    use crate::log::Metadata;
    use crate::proxy::channel::{ChannelTarget, ChannelTargetBuilder};
    use crate::proxy::format::Format;
    use crate::proxy::output::OutputSender;
    use chrono::TimeDelta;
    use serde_json::json;
    use std::cell::RefCell;
    use std::time::Instant;

    #[derive(Debug, Default)]
    struct RecordingLog {
        outputs: RefCell<Vec<Output>>,
        metadata: RefCell<Vec<Metadata>>,
    }

    impl RunLogWriter for RecordingLog {
        fn push_input(&self, _input: Input) {}
        fn push_output(&self, output: Output) {
            self.outputs.borrow_mut().push(output);
        }
        fn push_metadata(&self, metadata: Metadata) {
            self.metadata.borrow_mut().push(metadata);
        }
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

    fn recording_proxy(builder: ProxyBuilder, sent: &Rc<RefCell<Vec<Value>>>) -> Proxy {
        let mut builder = builder;
        builder.with_channel("logger", |c| {
            c.target(RecordingTargetBuilder(sent.clone()))
                .effects(vec!["ping".into()])
        });
        builder.build().unwrap()
    }

    #[test]
    fn realtime_holds_future_inputs_until_due() {
        let sent = Rc::new(RefCell::new(vec![]));
        let waits = Rc::new(RefCell::new(vec![]));
        let waits_hook = waits.clone();
        let builder = Proxy::builder()
            .realtime(true)
            .on_wait(move |until| waits_hook.borrow_mut().push(until));
        let mut proxy = recording_proxy(builder, &sent);

        let mut past = input(1);
        past.timestamp -= TimeDelta::seconds(10);
        let mut future = input(2);
        future.timestamp += TimeDelta::milliseconds(300);

        let started = Instant::now();
        proxy.send(&past).unwrap();
        assert!(waits.borrow().is_empty(), "past inputs aren't held");
        proxy.send(&future).unwrap();

        assert!(Utc::now() >= future.timestamp.to_utc());
        assert!(started.elapsed() >= Duration::from_millis(250));
        assert_eq!(waits.borrow().first(), Some(&future.timestamp));
        assert_eq!(sent.borrow().len(), 2);
    }

    #[test]
    fn stop_interrupts_a_wait_and_ignores_later_inputs() {
        let sent = Rc::new(RefCell::new(vec![]));
        let mut proxy = recording_proxy(Proxy::builder().realtime(true), &sent);
        let handle = proxy.stop_handle();

        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            handle.stop();
        });

        let mut future = input(1);
        future.timestamp += TimeDelta::minutes(10);
        let started = Instant::now();
        proxy.send(&future).unwrap();
        stopper.join().unwrap();

        assert!(started.elapsed() < Duration::from_secs(5));
        proxy.send(&input(2)).unwrap();
        assert!(sent.borrow().is_empty());
    }

    #[test]
    fn stop_handle_can_be_shared_before_build() {
        let sent = Rc::new(RefCell::new(vec![]));
        let handle = StopHandle::new();
        let mut proxy = recording_proxy(Proxy::builder().stop_handle(handle.clone()), &sent);

        proxy.send(&input(1)).unwrap();
        handle.stop();
        proxy.send(&input(2)).unwrap();

        assert_eq!(sent.borrow().len(), 1);
        assert!(proxy.stop_handle().is_stopped());
    }

    #[test]
    fn records_start_and_end_timing() {
        let log = Rc::new(RecordingLog::default());
        let mut proxy = Proxy::builder()
            .run_log_writer(log.clone())
            .build()
            .unwrap();

        proxy.finish();
        assert!(log.metadata.borrow().is_empty(), "nothing sent, no timing");

        proxy.send(&input(1)).unwrap();
        proxy.finish();
        proxy.finish();

        let kinds: Vec<_> = log
            .metadata
            .borrow()
            .iter()
            .map(|m| match &m.data {
                Some(data) if m.mtype == "timing" => data["key"].as_str().unwrap().to_string(),
                _ => m.mtype.clone(),
            })
            .collect();
        assert_eq!(kinds, ["simulation_start", "simulation_end"]);
    }
}
