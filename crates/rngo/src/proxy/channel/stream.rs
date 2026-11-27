use crate::effect::Input;
use crate::error::{BuildErrors, ParseErrors};
use crate::proxy::channel::{
    Channel, ChannelBuildContext, ChannelBuilder, ChannelParseContext, ChannelParser, SendContext,
    StderrTail,
};
use crate::proxy::format::{Format, FormatBuilder};
use crate::proxy::output::ChannelOutput;
use crate::{BuildError, ChannelError, ParseError, spec};
use serde_json::Value;
use std::io::Write;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const STARTUP_GRACE: Duration = Duration::from_millis(250);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(200);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Debug)]
pub(crate) struct Stream {
    channel_key: String,
    format: Option<Arc<dyn Format>>,
    child: Child,
    stdin: Option<ChildStdin>,
    reader_threads: Vec<JoinHandle<()>>,
    stderr_tail: StderrTail,
    startup_checked: bool,
}

impl Stream {
    fn exit_error(&mut self, grace: Duration) -> Option<ChannelError> {
        let deadline = Instant::now() + grace;
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(SHUTDOWN_POLL_INTERVAL),
                _ => return None,
            }
        };
        thread::sleep(SHUTDOWN_POLL_INTERVAL * 4);
        Some(ChannelError::exited(
            self.channel_key.clone(),
            status.to_string(),
            self.stderr_tail.text(),
        ))
    }

    pub(crate) fn parser() -> StreamParser {
        StreamParser {}
    }

    pub(crate) fn builder() -> StreamBuilder {
        StreamBuilder::default()
    }
}

impl Channel for Stream {
    fn send(
        &mut self,
        input: &Input,
        context: &SendContext,
    ) -> Result<Vec<ChannelOutput>, ChannelError> {
        if !self.startup_checked {
            self.startup_checked = true;
            if let Some(error) = self.exit_error(STARTUP_GRACE) {
                return Err(error);
            }
        }

        let Some(stdin) = self.stdin.as_mut() else {
            return Ok(vec![]);
        };

        let data = match super::render(self.format.as_deref(), input, context) {
            Ok(data) => data,
            Err(output) => return Ok(vec![output]),
        };

        let data = match data {
            Value::String(s) => s,
            other => other.to_string(),
        };
        if let Err(source) = writeln!(stdin, "{data}") {
            if source.kind() == std::io::ErrorKind::BrokenPipe
                && let Some(error) = self.exit_error(SHUTDOWN_GRACE)
            {
                return Err(error);
            }
            return Err(ChannelError::write(self.channel_key.clone(), source));
        }

        Ok(vec![])
    }

    fn finish(&mut self) -> Result<(), ChannelError> {
        self.stdin.take();

        let deadline = Instant::now() + SHUTDOWN_GRACE;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) if Instant::now() >= deadline => break,
                Ok(None) => thread::sleep(SHUTDOWN_POLL_INTERVAL),
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();

        super::join_readers(&mut self.reader_threads);
        Ok(())
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[derive(Default)]
pub struct StreamBuilder {
    command: Option<String>,
    format: Option<Box<dyn FormatBuilder>>,
}

impl StreamBuilder {
    pub fn format(mut self, format: impl FormatBuilder + 'static) -> Self {
        self.format = Some(Box::new(format));
        self
    }

    pub fn command(mut self, value: impl Into<String>) -> Self {
        self.command = Some(value.into());
        self
    }
}

impl ChannelBuilder for StreamBuilder {
    fn build(
        self: Box<Self>,
        context: ChannelBuildContext,
    ) -> Result<Box<dyn Channel>, BuildErrors> {
        let channel_key = context.key();
        let outputs = context.outputs();
        let Some(command) = self.command else {
            return Err(BuildError::channel(channel_key, "command not specified").into());
        };
        let format = super::build_format(self.format, &context)?;

        let mut child = super::shell(&command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                vec![BuildError::channel(
                    channel_key,
                    format!("failed to spawn command: {e}"),
                )]
            })?;

        let stdin = child.stdin.take();

        let stderr_tail = StderrTail::default();
        let reader_threads = super::spawn_readers(&mut child, &outputs, &stderr_tail);

        Ok(Box::new(Stream {
            channel_key: channel_key.to_string(),
            format,
            child,
            stdin,
            reader_threads,
            stderr_tail,
            startup_checked: false,
        }))
    }
}

pub(crate) struct StreamParser {}

impl ChannelParser for StreamParser {
    fn key(&self) -> &str {
        "stream"
    }

    fn parse(
        &self,
        channel: &spec::Channel,
        context: &ChannelParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, ParseErrors> {
        let command = match channel.fields.get("command") {
            Some(value) => match value.as_str() {
                Some(command) => command,
                None => {
                    return Err(ParseError::invalid("must be a string")
                        .at(["command"])
                        .into());
                }
            },
            None => {
                return Err(ParseError::invalid("is required for a stream channel")
                    .at(["command"])
                    .into());
            }
        };

        let mut builder = StreamBuilder::default().command(command);

        if let Some(value) = channel.fields.get("format")
            && let Some(format) = context.parse_format(&["format"], value)?
        {
            builder = builder.format(format);
        }

        Ok(Box::new(builder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::output::OutputSender;
    use jiff::Timestamp;
    use std::sync::mpsc;

    fn outputs() -> OutputSender {
        let (tx, _rx) = mpsc::channel();
        OutputSender::new("logger", tx)
    }

    #[test]
    fn builder_without_a_command_is_an_error() {
        let result =
            Box::new(StreamBuilder::default()).build(ChannelBuildContext::new("logger", outputs()));
        assert!(result.is_err());
    }

    #[test]
    fn builder_with_no_channel_key_at_construction_still_builds() {
        let result = Box::new(Stream::builder().command("cat"))
            .build(ChannelBuildContext::new("logger", outputs()));
        assert!(result.is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn drop_abandons_a_reader_thread_stuck_on_an_orphaned_grandchild() {
        let stream = Box::new(
            StreamBuilder::default()
                // Backgrounds `sleep 5` without redirecting it, so it inherits this shell's stdout
                // pipe and keeps it open (reparented, once this shell exits) well past both grace
                // periods below - simulating a command whose kill doesn't actually close its pipe.
                .command("sleep 5 & echo done".to_string()),
        )
        .build(ChannelBuildContext::new("test", outputs()))
        .unwrap();

        let start = Instant::now();
        drop(stream);
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_secs(2),
            "drop should abandon a reader thread stuck on an orphaned pipe rather than block \
             on it for the grandchild's full lifetime, took {elapsed:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn write_to_a_command_that_exited_reports_its_status_and_stderr() {
        let mut stream = Box::new(
            StreamBuilder::default().command("echo 'connection refused' >&2; exit 2".to_string()),
        )
        .build(ChannelBuildContext::new("db", outputs()))
        .unwrap();
        let input = Input {
            channel: None,
            id: 1,
            effect: "ping".into(),
            timestamp: Timestamp::now(),
            data: Value::String("x".into()),
            metadata: vec![],
        };

        let mut message = String::new();
        for _ in 0..50 {
            if let Err(error) = stream.send(&input, &SendContext::new()) {
                message = error.to_string();
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(message.contains("exit status: 2"), "{message}");
        assert!(message.contains("connection refused"), "{message}");
    }
}
