pub mod channel;
pub mod format;
pub mod output;
mod stop;

use crate::{BuildError, Input, Metadata, RunLogWriter, SimpleEventRunLog};
use channel::{Channel, ChannelBuilder};
use chrono::{DateTime, FixedOffset, Utc};
use output::{Output, OutputSender};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
pub use stop::StopHandle;

const WAIT_TICK: Duration = Duration::from_secs(1);

type OnWait = Box<dyn FnMut(DateTime<FixedOffset>)>;

pub struct Proxy {
    run_log_writer: Rc<dyn RunLogWriter>,
    channels: HashMap<String, Box<dyn Channel>>,
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

    /// Logs `input` and sends it to its effect's channel. In realtime mode, first waits until its
    /// timestamp. Once stopped, does nothing. The first call records the `simulation_start` timing.
    pub fn send(&mut self, input: &Input) -> Result<(), Box<dyn std::error::Error>> {
        if !self.started {
            self.started = true;
            self.record_timing("simulation_start");
        }

        if self.stop.is_stopped() || (self.realtime && !self.wait_until(input.timestamp)) {
            return Ok(());
        }

        self.run_log_writer.push_input(input.clone());

        let channel_key = match self.effect_channels.get(&input.effect) {
            Some(k) => k,
            None => return Ok(()),
        };

        if let Some(channel) = self.channels.get_mut(channel_key) {
            for output in channel.send(input)? {
                self.run_log_writer
                    .push_output(output.into_output(channel_key, Some(input.id)));
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

    /// Records the `simulation_end` timing (if anything was sent) and drains the channels.
    pub fn finish(&mut self) {
        if self.started && !self.finished {
            self.finished = true;
            self.record_timing("simulation_end");
        }

        for channel in self.channels.values_mut() {
            channel.finish();
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
    channel_builders: Vec<(String, Box<dyn ChannelBuilder>)>,
    effect_channels: HashMap<String, String>,
    realtime: bool,
    on_wait: Option<OnWait>,
    stop: StopHandle,
}

impl ProxyBuilder {
    pub fn new() -> Self {
        Self {
            run_log_writer: None,
            channel_builders: vec![],
            effect_channels: HashMap::new(),
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

    /// Adds a channel under `key`.
    pub fn channel(
        mut self,
        key: impl Into<String>,
        builder: impl ChannelBuilder + 'static,
    ) -> Self {
        self.set_channel(key, Box::new(builder));
        self
    }

    pub fn set_channel(
        &mut self,
        key: impl Into<String>,
        builder: Box<dyn ChannelBuilder>,
    ) -> &mut Self {
        self.channel_builders.push((key.into(), builder));
        self
    }

    /// Sends the inputs of `effects` to `channel`; an effect assigned twice goes to the last
    /// channel it was assigned to. `build` fails if `channel` was never added.
    pub fn channel_effects(
        mut self,
        channel: impl Into<String>,
        effects: &[impl AsRef<str>],
    ) -> Self {
        self.set_channel_effects(channel, effects);
        self
    }

    pub fn set_channel_effects(
        &mut self,
        channel: impl Into<String>,
        effects: &[impl AsRef<str>],
    ) -> &mut Self {
        let channel = channel.into();
        for effect in effects {
            self.effect_channels
                .insert(effect.as_ref().to_string(), channel.clone());
        }
        self
    }

    pub fn build(self) -> Result<Proxy, Vec<BuildError>> {
        let mut errors = vec![];
        let mut channels = HashMap::new();
        let (output_tx, output_rx) = mpsc::channel::<Output>();

        let run_log_writer = self
            .run_log_writer
            .unwrap_or_else(|| SimpleEventRunLog::new());

        let mut unknown: Vec<_> = self
            .effect_channels
            .values()
            .filter(|channel| !self.channel_builders.iter().any(|(key, _)| key == *channel))
            .collect();
        unknown.sort();
        unknown.dedup();
        for channel in unknown {
            errors.push(BuildError::Proxy {
                message: format!("effects were assigned to unknown channel: {channel}"),
            });
        }

        for (key, channel_builder) in self.channel_builders {
            let outputs = OutputSender::new(&key, output_tx.clone());

            match channel_builder.build(&key, outputs) {
                Ok(channel) => {
                    channels.insert(key, channel);
                }
                Err(mut e) => errors.append(&mut e),
            };
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(Proxy {
            run_log_writer,
            channels,
            effect_channels: self.effect_channels,
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
    use crate::Level;
    use crate::log::Metadata;
    use crate::proxy::channel::{Channel, ChannelBuilder, render};
    use crate::proxy::format::Format;
    use crate::proxy::output::{ChannelOutput, OutputSender};
    use chrono::TimeDelta;
    use serde_json::{Value, json};
    use std::cell::RefCell;
    use std::time::Instant;

    #[derive(Debug, Default)]
    struct RecordingLog {
        inputs: RefCell<Vec<Input>>,
        outputs: RefCell<Vec<Output>>,
        metadata: RefCell<Vec<Metadata>>,
    }

    impl RunLogWriter for RecordingLog {
        fn push_input(&self, input: Input) {
            self.inputs.borrow_mut().push(input);
        }
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
    struct RecordingSink {
        sent: Rc<RefCell<Vec<Value>>>,
        format: Option<Rc<dyn Format>>,
    }

    impl Channel for RecordingSink {
        fn send(
            &mut self,
            input: &Input,
        ) -> Result<Vec<ChannelOutput>, Box<dyn std::error::Error>> {
            match render(self.format.as_deref(), input) {
                Ok(data) => {
                    self.sent.borrow_mut().push(data);
                    Ok(vec![])
                }
                Err(output) => Ok(vec![output]),
            }
        }
    }

    struct RecordingSinkBuilder {
        sent: Rc<RefCell<Vec<Value>>>,
        format: Option<Rc<dyn Format>>,
    }

    impl RecordingSinkBuilder {
        fn new(sent: &Rc<RefCell<Vec<Value>>>) -> Self {
            RecordingSinkBuilder {
                sent: sent.clone(),
                format: None,
            }
        }
    }

    impl ChannelBuilder for RecordingSinkBuilder {
        fn build(
            &self,
            _channel_key: &str,
            _outputs: OutputSender,
        ) -> Result<Box<dyn Channel>, Vec<BuildError>> {
            Ok(Box::new(RecordingSink {
                sent: self.sent.clone(),
                format: self.format.clone(),
            }))
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
        let mut channel = RecordingSinkBuilder::new(&sent);
        channel.format = Some(Rc::new(FailOnEvenIds));
        let mut proxy = Proxy::builder()
            .run_log_writer(log.clone())
            .channel("logger", channel)
            .channel_effects("logger", &["ping"])
            .build()
            .unwrap();

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
        let mut proxy = Proxy::builder()
            .channel("logger", RecordingSinkBuilder::new(&sent))
            .channel_effects("logger", &["ping"])
            .build()
            .unwrap();

        let mut ping = input(1);
        ping.data = json!({ "a": 1 });
        proxy.send(&ping).unwrap();

        assert_eq!(*sent.borrow(), vec![json!({ "a": 1 })]);
    }

    #[test]
    fn channel_outputs_are_tied_to_the_input() {
        let log = Rc::new(RecordingLog::default());
        let mut proxy = Proxy::builder()
            .run_log_writer(log.clone())
            .channel("logger", channel::Exec::builder())
            .channel_effects("logger", &["ping"])
            .build()
            .unwrap();

        let mut ping = input(5);
        ping.data = json!("echo hi");
        proxy.send(&ping).unwrap();

        let outputs = log.outputs.borrow();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].input_id, Some(5));
        assert_eq!(outputs[0].data, "hi");
    }

    fn recording_proxy(builder: ProxyBuilder, sent: &Rc<RefCell<Vec<Value>>>) -> Proxy {
        builder
            .channel("logger", RecordingSinkBuilder::new(sent))
            .channel_effects("logger", &["ping"])
            .build()
            .unwrap()
    }

    #[test]
    fn build_passes_the_channel_key_to_its_builder() {
        struct KeyRecorder(Rc<RefCell<Option<String>>>);

        impl ChannelBuilder for KeyRecorder {
            fn build(
                &self,
                channel_key: &str,
                _outputs: OutputSender,
            ) -> Result<Box<dyn Channel>, Vec<BuildError>> {
                *self.0.borrow_mut() = Some(channel_key.to_string());
                Ok(Box::new(RecordingSink {
                    sent: Rc::new(RefCell::new(vec![])),
                    format: None,
                }))
            }
        }

        let seen = Rc::new(RefCell::new(None));
        Proxy::builder()
            .channel("db", KeyRecorder(seen.clone()))
            .build()
            .unwrap();

        assert_eq!(seen.borrow().as_deref(), Some("db"));
    }

    #[test]
    fn effects_assigned_to_an_unknown_channel_fail_the_build() {
        let result = Proxy::builder().channel_effects("nope", &["ping"]).build();

        assert!(matches!(
            result.err().as_deref(),
            Some([BuildError::Proxy { message }]) if message.contains("nope")
        ));
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

    #[test]
    fn logs_inputs_unless_stopped() {
        let log = Rc::new(RecordingLog::default());
        let mut proxy = Proxy::builder()
            .run_log_writer(log.clone())
            .build()
            .unwrap();

        proxy.send(&input(1)).unwrap();
        proxy.stop_handle().stop();
        proxy.send(&input(2)).unwrap();

        let ids: Vec<_> = log.inputs.borrow().iter().map(|i| i.id).collect();
        assert_eq!(ids, [1]);
    }
}
