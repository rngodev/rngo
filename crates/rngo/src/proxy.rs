pub mod channel;
pub mod format;
pub(crate) mod output;
mod stop;

use crate::error::BuildErrors;
use crate::{Input, LogError, LogWriter, Metadata, ProxyError, SimpleLog};
use channel::{Channel, ChannelBuildContext, ChannelBuilder, SendContext};
use jiff::Timestamp;
use output::{Output, OutputSender};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
pub use stop::StopHandle;

const WAIT_TICK: Duration = Duration::from_secs(1);

type OnWait = Box<dyn FnMut(Timestamp) + Send>;

pub struct Proxy {
    log_writer: Arc<dyn LogWriter>,
    channels: HashMap<String, Box<dyn Channel>>,
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

    /// Logs `input` and sends it to its channel ([`Input::channel`]), if it has one. In realtime
    /// mode, first waits until its timestamp. Once stopped, does nothing. The first call records the
    /// `simulation_start` timing. An input naming a channel the proxy doesn't have is
    /// [`ProxyError::UnknownChannel`], and isn't logged.
    pub fn send(&mut self, input: &Input) -> Result<(), ProxyError> {
        if !self.started {
            self.started = true;
            self.record_timing("simulation_start")?;
        }

        if self.stop.is_stopped() || (self.realtime && !self.wait_until(input.timestamp)) {
            return Ok(());
        }

        if let Some(channel_key) = &input.channel
            && !self.channels.contains_key(channel_key)
        {
            return Err(ProxyError::UnknownChannel {
                effect: input.effect.clone(),
                channel: channel_key.clone(),
            });
        }

        self.log_writer.push_input(input.clone())?;

        if let Some(channel_key) = &input.channel
            && let Some(channel) = self.channels.get_mut(channel_key)
        {
            for output in channel.send(input, &SendContext::new())? {
                self.log_writer
                    .push_output(output.into_output(channel_key, Some(input.id)))?;
            }
        }

        self.drain_outputs()?;
        Ok(())
    }

    /// A handle that can stop this proxy from any thread.
    pub fn stop_handle(&self) -> StopHandle {
        self.stop.clone()
    }

    /// Blocks until `timestamp` has passed, calling `on_wait` about once a second meanwhile;
    /// returns `false` if stopped first.
    fn wait_until(&mut self, timestamp: Timestamp) -> bool {
        loop {
            let remaining = Timestamp::now().duration_until(timestamp);
            if !remaining.is_positive() {
                return true;
            }
            let remaining = remaining.unsigned_abs();

            if let Some(on_wait) = self.on_wait.as_mut() {
                on_wait(timestamp);
            }

            if self.stop.wait_timeout(remaining.min(WAIT_TICK)) {
                return false;
            }
        }
    }

    /// Records the `simulation_end` timing (if anything was sent) and drains the channels.
    pub fn finish(&mut self) -> Result<(), LogError> {
        if self.started && !self.finished {
            self.finished = true;
            self.record_timing("simulation_end")?;
        }

        for channel in self.channels.values_mut() {
            channel.finish();
        }
        self.drain_outputs()
    }

    fn record_timing(&self, key: &str) -> Result<(), LogError> {
        self.log_writer
            .push_metadata(Metadata::new("timing").data(serde_json::json!({
                "key": key,
                "timestamp": Timestamp::now().as_millisecond(),
            })))
    }

    fn drain_outputs(&mut self) -> Result<(), LogError> {
        while let Ok(output) = self.output_rx.try_recv() {
            self.log_writer.push_output(output)?;
        }
        Ok(())
    }
}

pub struct ProxyBuilder {
    log_writer: Option<Arc<dyn LogWriter>>,
    channel_builders: Vec<(String, Box<dyn ChannelBuilder>)>,
    realtime: bool,
    on_wait: Option<OnWait>,
    stop: StopHandle,
}

impl ProxyBuilder {
    pub fn new() -> Self {
        Self {
            log_writer: None,
            channel_builders: vec![],
            realtime: false,
            on_wait: None,
            stop: StopHandle::new(),
        }
    }

    pub fn log_writer<T: LogWriter + 'static>(mut self, writer: Arc<T>) -> Self {
        self.log_writer = Some(writer as Arc<dyn LogWriter>);
        self
    }

    /// When true, holds each input until its timestamp has passed; otherwise sends it right away.
    pub fn realtime(mut self, value: bool) -> Self {
        self.realtime = value;
        self
    }

    /// Called with the awaited timestamp when a realtime wait starts and about once a second
    /// while it lasts.
    pub fn on_wait(mut self, f: impl FnMut(Timestamp) + Send + 'static) -> Self {
        self.on_wait = Some(Box::new(f));
        self
    }

    /// Uses `handle` to stop the built proxy, so it can be shared before the proxy exists.
    pub fn stop_handle(mut self, handle: StopHandle) -> Self {
        self.stop = handle;
        self
    }

    /// Adds a channel under `key`.
    pub fn channel(
        mut self,
        key: impl Into<String>,
        builder: impl ChannelBuilder + 'static,
    ) -> Self {
        self.channel_builders.push((key.into(), Box::new(builder)));
        self
    }

    pub fn build(self) -> Result<Proxy, BuildErrors> {
        let mut errors = vec![];
        let mut channels = HashMap::new();
        let (output_tx, output_rx) = mpsc::channel::<Output>();

        let log_writer = self
            .log_writer
            .unwrap_or_else(|| Arc::new(SimpleLog::new()));

        for (key, channel_builder) in self.channel_builders {
            let outputs = OutputSender::new(&key, output_tx.clone());

            match channel_builder.build(ChannelBuildContext::new(&key, outputs)) {
                Ok(channel) => {
                    channels.insert(key, channel);
                }
                Err(e) => errors.extend(e),
            };
        }

        if !errors.is_empty() {
            return Err(errors.into());
        }

        Ok(Proxy {
            log_writer,
            channels,
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
    use crate::proxy::output::ChannelOutput;
    use jiff::{SignedDuration, Timestamp};
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use std::time::Instant;

    #[derive(Debug, Default)]
    struct RecordingLog {
        inputs: Mutex<Vec<Input>>,
        outputs: Mutex<Vec<Output>>,
        metadata: Mutex<Vec<Metadata>>,
    }

    impl LogWriter for RecordingLog {
        fn push_input(&self, input: Input) -> Result<(), LogError> {
            self.inputs.lock().unwrap().push(input);
            Ok(())
        }
        fn push_output(&self, output: Output) -> Result<(), LogError> {
            self.outputs.lock().unwrap().push(output);
            Ok(())
        }
        fn push_metadata(&self, metadata: Metadata) -> Result<(), LogError> {
            self.metadata.lock().unwrap().push(metadata);
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FailOnEvenIds;

    impl Format for FailOnEvenIds {
        fn format(
            &self,
            event: &Input,
            _context: &crate::format::FormatContext,
        ) -> Result<String, crate::FormatError> {
            if event.id.is_multiple_of(2) {
                Err(crate::FormatError::Render("boom".into()))
            } else {
                Ok(event.id.to_string())
            }
        }
    }

    #[derive(Debug)]
    struct RecordingSink {
        sent: Arc<Mutex<Vec<Value>>>,
        format: Option<Arc<dyn Format>>,
    }

    impl Channel for RecordingSink {
        fn send(
            &mut self,
            input: &Input,
            context: &SendContext,
        ) -> Result<Vec<ChannelOutput>, crate::ChannelError> {
            match render(self.format.as_deref(), input, context) {
                Ok(data) => {
                    self.sent.lock().unwrap().push(data);
                    Ok(vec![])
                }
                Err(output) => Ok(vec![output]),
            }
        }
    }

    struct RecordingSinkBuilder {
        sent: Arc<Mutex<Vec<Value>>>,
        format: Option<Arc<dyn Format>>,
    }

    impl RecordingSinkBuilder {
        fn new(sent: &Arc<Mutex<Vec<Value>>>) -> Self {
            RecordingSinkBuilder {
                sent: sent.clone(),
                format: None,
            }
        }
    }

    impl ChannelBuilder for RecordingSinkBuilder {
        fn build(
            self: Box<Self>,
            _context: ChannelBuildContext,
        ) -> Result<Box<dyn Channel>, BuildErrors> {
            Ok(Box::new(RecordingSink {
                sent: self.sent,
                format: self.format,
            }))
        }
    }

    fn input(id: u64) -> Input {
        Input {
            id,
            effect: "ping".into(),
            timestamp: Timestamp::now(),
            data: json!(null),
            metadata: vec![],
            channel: Some("logger".into()),
        }
    }

    /// An input with no channel, which the proxy only logs.
    fn unrouted(id: u64) -> Input {
        Input {
            channel: None,
            ..input(id)
        }
    }

    #[test]
    fn format_failure_records_an_error_output_and_keeps_going() {
        let log = Arc::new(RecordingLog::default());
        let sent = Arc::new(Mutex::new(vec![]));
        let mut channel = RecordingSinkBuilder::new(&sent);
        channel.format = Some(Arc::new(FailOnEvenIds));
        let mut proxy = Proxy::builder()
            .log_writer(log.clone())
            .channel("logger", channel)
            .build()
            .unwrap();

        for id in 1..=3 {
            proxy.send(&input(id)).unwrap();
        }

        assert_eq!(*sent.lock().unwrap(), vec![json!("1"), json!("3")]);
        let outputs = log.outputs.lock().unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].input_id, Some(2));
        assert_eq!(outputs[0].channel, "logger");
        assert!(matches!(outputs[0].level, Level::Error));
        assert!(outputs[0].data.contains("boom"));
    }

    #[test]
    fn unformatted_channels_send_the_input_data() {
        let sent = Arc::new(Mutex::new(vec![]));
        let mut proxy = Proxy::builder()
            .channel("logger", RecordingSinkBuilder::new(&sent))
            .build()
            .unwrap();

        let mut ping = input(1);
        ping.data = json!({ "a": 1 });
        proxy.send(&ping).unwrap();

        assert_eq!(*sent.lock().unwrap(), vec![json!({ "a": 1 })]);
    }

    #[test]
    fn channel_outputs_are_tied_to_the_input() {
        let log = Arc::new(RecordingLog::default());
        let mut proxy = Proxy::builder()
            .log_writer(log.clone())
            .channel("logger", channel::Exec::builder())
            .build()
            .unwrap();

        let mut ping = input(5);
        ping.data = json!("echo hi");
        proxy.send(&ping).unwrap();

        let outputs = log.outputs.lock().unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].input_id, Some(5));
        assert_eq!(outputs[0].data, "hi");
    }

    fn recording_proxy(builder: ProxyBuilder, sent: &Arc<Mutex<Vec<Value>>>) -> Proxy {
        builder
            .channel("logger", RecordingSinkBuilder::new(sent))
            .build()
            .unwrap()
    }

    #[test]
    fn build_passes_the_channel_key_to_its_builder() {
        struct KeyRecorder(Arc<Mutex<Option<String>>>);

        impl ChannelBuilder for KeyRecorder {
            fn build(
                self: Box<Self>,
                context: ChannelBuildContext,
            ) -> Result<Box<dyn Channel>, BuildErrors> {
                *self.0.lock().unwrap() = Some(context.key().to_string());
                Ok(Box::new(RecordingSink {
                    sent: Arc::new(Mutex::new(vec![])),
                    format: None,
                }))
            }
        }

        let seen = Arc::new(Mutex::new(None));
        Proxy::builder()
            .channel("db", KeyRecorder(seen.clone()))
            .build()
            .unwrap();

        assert_eq!(seen.lock().unwrap().as_deref(), Some("db"));
    }

    #[test]
    fn an_input_for_an_unknown_channel_is_an_error_and_is_not_logged() {
        let log = Arc::new(RecordingLog::default());
        let mut proxy = Proxy::builder().log_writer(log.clone()).build().unwrap();

        let result = proxy.send(&input(1).channel("nope"));

        assert!(matches!(
            result,
            Err(ProxyError::UnknownChannel { ref effect, ref channel, .. })
                if effect == "ping" && channel == "nope"
        ));
        assert!(log.inputs.lock().unwrap().is_empty());
    }

    #[test]
    fn an_input_with_no_channel_is_logged_but_not_sent() {
        let log = Arc::new(RecordingLog::default());
        let sent = Arc::new(Mutex::new(vec![]));
        let mut proxy = Proxy::builder()
            .log_writer(log.clone())
            .channel("logger", RecordingSinkBuilder::new(&sent))
            .build()
            .unwrap();

        proxy.send(&unrouted(1)).unwrap();

        assert_eq!(log.inputs.lock().unwrap().len(), 1);
        assert!(sent.lock().unwrap().is_empty());
    }

    #[test]
    fn realtime_holds_future_inputs_until_due() {
        let sent = Arc::new(Mutex::new(vec![]));
        let waits = Arc::new(Mutex::new(vec![]));
        let waits_hook = waits.clone();
        let builder = Proxy::builder()
            .realtime(true)
            .on_wait(move |until| waits_hook.lock().unwrap().push(until));
        let mut proxy = recording_proxy(builder, &sent);

        let mut past = input(1);
        past.timestamp -= SignedDuration::from_secs(10);
        let mut future = input(2);
        future.timestamp += SignedDuration::from_millis(300);

        let started = Instant::now();
        proxy.send(&past).unwrap();
        assert!(waits.lock().unwrap().is_empty(), "past inputs aren't held");
        proxy.send(&future).unwrap();

        assert!(Timestamp::now() >= future.timestamp);
        assert!(started.elapsed() >= Duration::from_millis(250));
        assert_eq!(waits.lock().unwrap().first(), Some(&future.timestamp));
        assert_eq!(sent.lock().unwrap().len(), 2);
    }

    #[test]
    fn stop_interrupts_a_wait_and_ignores_later_inputs() {
        let sent = Arc::new(Mutex::new(vec![]));
        let mut proxy = recording_proxy(Proxy::builder().realtime(true), &sent);
        let handle = proxy.stop_handle();

        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            handle.stop();
        });

        let mut future = input(1);
        future.timestamp += SignedDuration::from_mins(10);
        let started = Instant::now();
        proxy.send(&future).unwrap();
        stopper.join().unwrap();

        assert!(started.elapsed() < Duration::from_secs(5));
        proxy.send(&input(2)).unwrap();
        assert!(sent.lock().unwrap().is_empty());
    }

    #[test]
    fn stop_handle_can_be_shared_before_build() {
        let sent = Arc::new(Mutex::new(vec![]));
        let handle = StopHandle::new();
        let mut proxy = recording_proxy(Proxy::builder().stop_handle(handle.clone()), &sent);

        proxy.send(&input(1)).unwrap();
        handle.stop();
        proxy.send(&input(2)).unwrap();

        assert_eq!(sent.lock().unwrap().len(), 1);
        assert!(proxy.stop_handle().is_stopped());
    }

    #[test]
    fn records_start_and_end_timing() {
        let log = Arc::new(RecordingLog::default());
        let mut proxy = Proxy::builder().log_writer(log.clone()).build().unwrap();

        proxy.finish().unwrap();
        assert!(
            log.metadata.lock().unwrap().is_empty(),
            "nothing sent, no timing"
        );

        proxy.send(&unrouted(1)).unwrap();
        proxy.finish().unwrap();
        proxy.finish().unwrap();

        let kinds: Vec<_> = log
            .metadata
            .lock()
            .unwrap()
            .iter()
            .map(|m| match &m.data {
                Some(data) if m.kind == "timing" => data["key"].as_str().unwrap().to_string(),
                _ => m.kind.clone(),
            })
            .collect();
        assert_eq!(kinds, ["simulation_start", "simulation_end"]);
    }

    #[test]
    fn logs_inputs_unless_stopped() {
        let log = Arc::new(RecordingLog::default());
        let mut proxy = Proxy::builder().log_writer(log.clone()).build().unwrap();

        proxy.send(&unrouted(1)).unwrap();
        proxy.stop_handle().stop();
        proxy.send(&unrouted(2)).unwrap();

        let ids: Vec<_> = log.inputs.lock().unwrap().iter().map(|i| i.id).collect();
        assert_eq!(ids, [1]);
    }
}
