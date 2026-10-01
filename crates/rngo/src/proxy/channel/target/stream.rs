use crate::parse::ChannelTargetParser;
use crate::proxy::channel::{ChannelTarget, ChannelTargetBuilder};
use crate::{BuildError, Level, OutputSender, ParseError, TargetOutput, spec};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const SHUTDOWN_GRACE: Duration = Duration::from_millis(200);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(5);
const READER_JOIN_GRACE: Duration = Duration::from_millis(200);

#[derive(Debug)]
pub struct Stream {
    channel_key: String,
    child: Child,
    stdin: Option<ChildStdin>,
    reader_threads: Vec<JoinHandle<()>>,
}

impl Stream {
    pub fn parser() -> StreamParser {
        StreamParser {}
    }

    pub fn builder() -> StreamBuilder {
        StreamBuilder::default()
    }
}

impl ChannelTarget for Stream {
    fn send(&mut self, data: Value) -> Result<Vec<TargetOutput>, Box<dyn std::error::Error>> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Ok(vec![]);
        };

        let data = match data {
            Value::String(s) => s,
            other => other.to_string(),
        };
        writeln!(stdin, "{data}").map_err(|e| format!("channel '{}': {e}", self.channel_key))?;

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
}

impl StreamBuilder {
    pub fn command(mut self, value: impl Into<String>) -> Self {
        self.set_command(value);
        self
    }

    pub fn set_command(&mut self, value: impl Into<String>) -> &mut Self {
        self.command = Some(value.into());
        self
    }
}

impl ChannelTargetBuilder for StreamBuilder {
    fn build(
        &self,
        channel_key: &str,
        outputs: OutputSender,
    ) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>> {
        let Some(command) = self.command.clone() else {
            return Err(vec![BuildError::ChannelTarget {
                channel: channel_key.to_string(),
                message: "command not specified".into(),
            }]);
        };

        let mut child = super::shell(&command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                vec![BuildError::ChannelTarget {
                    channel: channel_key.to_string(),
                    message: format!("failed to spawn command: {e}"),
                }]
            })?;

        let stdin = child.stdin.take();

        let mut reader_threads = vec![];

        if let Some(stdout) = child.stdout.take() {
            reader_threads.push(spawn_reader(stdout, Level::Info, outputs.clone()));
        }

        if let Some(stderr) = child.stderr.take() {
            reader_threads.push(spawn_reader(stderr, Level::Error, outputs));
        }

        Ok(Box::new(Stream {
            channel_key: channel_key.to_string(),
            child,
            stdin,
            reader_threads,
        }))
    }
}

fn spawn_reader(
    pipe: impl std::io::Read + Send + 'static,
    level: Level,
    outputs: OutputSender,
) -> JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            if !line.is_empty() {
                outputs.send(TargetOutput::new(level, line));
            }
        }
    })
}

pub struct StreamParser {}

impl ChannelTargetParser for StreamParser {
    fn key(&self) -> &str {
        "stream"
    }

    fn parse(
        &self,
        channel_target: &spec::ChannelTarget,
    ) -> Result<Box<dyn ChannelTargetBuilder>, Vec<ParseError>> {
        let command = match channel_target.fields.get("command") {
            Some(value) => match value.as_str() {
                Some(command) => command,
                None => {
                    return Err(vec![ParseError::SchemaError {
                        path: None,
                        message: "FIX ME must be a string".into(),
                    }]);
                }
            },
            None => {
                return Err(vec![ParseError::SchemaError {
                    path: None,
                    message: "FIX ME must be a string".into(),
                }]);
            }
        };

        Ok(Box::new(StreamBuilder::default().command(command)))
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
}
