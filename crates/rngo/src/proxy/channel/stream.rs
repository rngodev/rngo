use crate::effect::Input;
use crate::parse::{ChannelParser, FormatParseContext};
use crate::proxy::channel::{Channel, ChannelBuilder};
use crate::proxy::format::Format;
use crate::proxy::output::{ChannelOutput, OutputSender};
use crate::{BuildError, ChannelError, Level, ParseError, spec};
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const STARTUP_GRACE: Duration = Duration::from_millis(250);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(200);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(5);
const READER_JOIN_GRACE: Duration = Duration::from_millis(200);
const STDERR_TAIL_LINES: usize = 5;

#[derive(Debug)]
pub struct Stream {
    channel_key: String,
    format: Option<Rc<dyn Format>>,
    child: Child,
    stdin: Option<ChildStdin>,
    reader_threads: Vec<JoinHandle<()>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
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
        let stderr = self
            .stderr_tail
            .lock()
            .map(|tail| tail.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        Some(ChannelError::Exited {
            channel: self.channel_key.clone(),
            status: status.to_string(),
            stderr,
        })
    }

    pub fn parser() -> StreamParser {
        StreamParser {}
    }

    pub fn builder() -> StreamBuilder {
        StreamBuilder::default()
    }
}

impl Channel for Stream {
    fn send(&mut self, input: &Input) -> Result<Vec<ChannelOutput>, ChannelError> {
        if !self.startup_checked {
            self.startup_checked = true;
            if let Some(error) = self.exit_error(STARTUP_GRACE) {
                return Err(error);
            }
        }

        let Some(stdin) = self.stdin.as_mut() else {
            return Ok(vec![]);
        };

        let data = match super::render(self.format.as_deref(), input) {
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
            return Err(ChannelError::Write {
                channel: self.channel_key.clone(),
                source,
            });
        }

        Ok(vec![])
    }

    fn finish(&mut self) {
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

        let deadline = Instant::now() + READER_JOIN_GRACE;
        for handle in self.reader_threads.drain(..) {
            while !handle.is_finished() && Instant::now() < deadline {
                thread::sleep(SHUTDOWN_POLL_INTERVAL);
            }
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.finish();
    }
}

#[derive(Default)]
pub struct StreamBuilder {
    command: Option<String>,
    format: Option<Rc<dyn Format>>,
}

impl StreamBuilder {
    pub fn format(mut self, format: impl Format + 'static) -> Self {
        self.set_format(Rc::new(format));
        self
    }

    pub fn set_format(&mut self, format: Rc<dyn Format>) -> &mut Self {
        self.format = Some(format);
        self
    }

    pub fn command(mut self, value: impl Into<String>) -> Self {
        self.set_command(value);
        self
    }

    pub fn set_command(&mut self, value: impl Into<String>) -> &mut Self {
        self.command = Some(value.into());
        self
    }
}

impl ChannelBuilder for StreamBuilder {
    fn build(
        &self,
        channel_key: &str,
        outputs: OutputSender,
    ) -> Result<Box<dyn Channel>, Vec<BuildError>> {
        let Some(command) = self.command.clone() else {
            return Err(vec![BuildError::channel(
                channel_key,
                "command not specified",
            )]);
        };

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

        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        let mut reader_threads = vec![];

        if let Some(stdout) = child.stdout.take() {
            reader_threads.push(spawn_reader(stdout, Level::Info, outputs.clone(), None));
        }

        if let Some(stderr) = child.stderr.take() {
            reader_threads.push(spawn_reader(
                stderr,
                Level::Error,
                outputs,
                Some(stderr_tail.clone()),
            ));
        }

        Ok(Box::new(Stream {
            channel_key: channel_key.to_string(),
            format: self.format.clone(),
            child,
            stdin,
            reader_threads,
            stderr_tail,
            startup_checked: false,
        }))
    }
}

fn spawn_reader(
    pipe: impl std::io::Read + Send + 'static,
    level: Level,
    outputs: OutputSender,
    tail: Option<Arc<Mutex<VecDeque<String>>>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            if !line.is_empty() {
                if let Some(tail) = &tail
                    && let Ok(mut tail) = tail.lock()
                {
                    if tail.len() == STDERR_TAIL_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line.clone());
                }
                outputs.send(ChannelOutput::new(level, line));
            }
        }
    })
}

pub struct StreamParser {}

impl ChannelParser for StreamParser {
    fn key(&self) -> &str {
        "stream"
    }

    fn parse(
        &self,
        channel: &spec::Channel,
        formats: &FormatParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, Vec<ParseError>> {
        let command = match channel.fields.get("command") {
            Some(value) => match value.as_str() {
                Some(command) => command,
                None => {
                    return Err(vec![
                        ParseError::invalid("must be a string").at(["command"]),
                    ]);
                }
            },
            None => {
                return Err(vec![
                    ParseError::invalid("is required for a stream channel").at(["command"]),
                ]);
            }
        };

        let mut builder = StreamBuilder::default().command(command);

        if let Some(value) = channel.fields.get("format")
            && let Some(format) = formats.parse(&["format"], value)?
        {
            builder.set_format(format);
        }

        Ok(Box::new(builder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn outputs() -> OutputSender {
        let (tx, _rx) = mpsc::channel();
        OutputSender::new("logger", tx)
    }

    #[test]
    fn builder_without_a_command_is_an_error() {
        let result = StreamBuilder::default().build("logger", outputs());
        assert!(result.is_err());
    }

    #[test]
    fn builder_with_no_channel_key_at_construction_still_builds() {
        let result = Stream::builder().command("cat").build("logger", outputs());
        assert!(result.is_ok());
    }

    #[test]
    fn drop_abandons_a_reader_thread_stuck_on_an_orphaned_grandchild() {
        let stream = StreamBuilder::default()
            // Backgrounds `sleep 5` without redirecting it, so it inherits this shell's stdout
            // pipe and keeps it open (reparented, once this shell exits) well past both grace
            // periods below - simulating a command whose kill doesn't actually close its pipe.
            .command("sleep 5 & echo done".to_string())
            .build("test", outputs())
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
    fn write_to_a_command_that_exited_reports_its_status_and_stderr() {
        let mut stream = StreamBuilder::default()
            .command("echo 'connection refused' >&2; exit 2".to_string())
            .build("db", outputs())
            .unwrap();
        let input = Input {
            id: 1,
            effect: "ping".into(),
            timestamp: chrono::Utc::now().fixed_offset(),
            data: Value::String("x".into()),
            metadata: vec![],
        };

        let mut message = String::new();
        for _ in 0..50 {
            if let Err(error) = stream.send(&input) {
                message = error.to_string();
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(message.contains("exit status: 2"), "{message}");
        assert!(message.contains("connection refused"), "{message}");
    }
}
