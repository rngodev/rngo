pub(crate) mod batch;
pub(crate) mod exec;
pub(crate) mod stdout;
pub(crate) mod stream;

pub(crate) use batch::Batch;
pub(crate) use exec::Exec;
pub(crate) use stdout::Stdout;
pub(crate) use stream::Stream;

mod parser;
pub use crate::proxy::output::{ChannelOutput, OutputSender};
pub use parser::{ChannelParseContext, ChannelParser};

use crate::effect::Input;
use crate::error::BuildErrors;
use crate::proxy::format::{Format, FormatBuildContext, FormatBuilder, FormatContext};
use crate::{ChannelError, Level};
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const STDERR_TAIL_LINES: usize = 5;
const READER_JOIN_GRACE: Duration = Duration::from_millis(200);
const READER_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Where a proxy delivers the inputs of the effects assigned to it.
pub trait Channel: std::fmt::Debug + Send {
    /// Sends one input, formatted by the channel's own format if it has one. The proxy ties
    /// returned outputs to the input.
    fn send(
        &mut self,
        input: &Input,
        context: &SendContext,
    ) -> Result<Vec<ChannelOutput>, ChannelError>;
    /// Delivers anything the channel still holds and stops it. Called once, when the proxy
    /// finishes; an error means some inputs never reached their destination.
    fn finish(&mut self) -> Result<(), ChannelError> {
        Ok(())
    }
}

/// What a channel sees when it sends an input.
pub struct SendContext<'a> {
    format_context: FormatContext<'a>,
}

impl<'a> SendContext<'a> {
    pub(crate) fn new() -> Self {
        SendContext {
            format_context: FormatContext::new(),
        }
    }

    /// The context to pass to the channel's format.
    pub fn format_context(&self) -> &FormatContext<'a> {
        &self.format_context
    }
}

/// Builds a [`Channel`] when its proxy is built. A builder gets the key it is added under only
/// then, so it can be made before the key is known.
pub trait ChannelBuilder: Send {
    fn build(
        self: Box<Self>,
        context: ChannelBuildContext,
    ) -> Result<Box<dyn Channel>, BuildErrors>;
}

/// What a channel builder gets when its proxy is built.
pub struct ChannelBuildContext<'a> {
    key: &'a str,
    outputs: OutputSender,
    dir: Option<&'a Path>,
    realtime: bool,
}

impl<'a> ChannelBuildContext<'a> {
    pub(crate) fn new(key: &'a str, outputs: OutputSender) -> Self {
        ChannelBuildContext {
            key,
            outputs,
            dir: None,
            realtime: false,
        }
    }

    pub(crate) fn with_dir(mut self, dir: Option<&'a Path>) -> Self {
        self.dir = dir;
        self
    }

    pub(crate) fn with_realtime(mut self, realtime: bool) -> Self {
        self.realtime = realtime;
        self
    }

    /// The directory the channel may write files in, if the proxy was given one
    /// ([`ProxyBuilder::dir`](crate::ProxyBuilder::dir)).
    pub fn dir(&self) -> Option<&'a Path> {
        self.dir
    }

    /// Whether the proxy sends each input only once its timestamp has passed.
    pub fn realtime(&self) -> bool {
        self.realtime
    }

    /// The channel's key, to name it in build errors.
    pub fn key(&self) -> &'a str {
        self.key
    }

    /// Reports outputs outside of [`Channel::send`], such as from a reader thread.
    pub fn outputs(&self) -> OutputSender {
        self.outputs.clone()
    }

    /// The context to build the channel's format with.
    pub fn format_context(&self) -> FormatBuildContext<'a> {
        FormatBuildContext::new(self.key)
    }
}

impl ChannelBuilder for Box<dyn ChannelBuilder> {
    fn build(
        self: Box<Self>,
        context: ChannelBuildContext,
    ) -> Result<Box<dyn Channel>, BuildErrors> {
        (*self).build(context)
    }
}

/// Builds a channel's optional format.
fn build_format(
    format: Option<Box<dyn FormatBuilder>>,
    context: &ChannelBuildContext,
) -> Result<Option<Arc<dyn Format>>, BuildErrors> {
    format
        .map(|format| format.build(context.format_context()).map(Arc::from))
        .transpose()
}

/// The value a channel delivers for `input`: its formatted string, or its own `data` without a
/// format. A format failure comes back as the error output to report instead.
pub(crate) fn render(
    format: Option<&dyn Format>,
    input: &Input,
    context: &SendContext,
) -> Result<Value, ChannelOutput> {
    match format.map(|f| f.format(input, context.format_context())) {
        Some(Ok(data)) => Ok(Value::String(data)),
        Some(Err(error)) => Err(ChannelOutput::new(
            Level::Error,
            format!("format failed: {error}"),
        )),
        None => Ok(input.data.clone()),
    }
}

/// Builds a process that runs `command` through the platform shell: `sh -c` on Unix, `cmd /C` on
/// Windows (so on Windows, commands are `cmd` syntax). It is placed in its own process group, so a
/// terminal Ctrl-C reaches rngo but not its channel subprocesses.
fn shell(command: &str) -> Command {
    #[cfg(unix)]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        cmd
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        let mut cmd = Command::new("cmd");
        // `raw_arg` so std doesn't re-quote the command; `cmd` parses its own command line.
        cmd.arg("/C").raw_arg(command);
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
        cmd
    }
}

/// The last lines a command wrote to stderr, kept for its exit error.
#[derive(Clone, Debug, Default)]
pub(crate) struct StderrTail(Arc<Mutex<VecDeque<String>>>);

impl StderrTail {
    fn push(&self, line: &str) {
        if let Ok(mut tail) = self.0.lock() {
            if tail.len() == STDERR_TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line.to_string());
        }
    }

    /// The kept lines, joined by newlines.
    pub(crate) fn text(&self) -> String {
        self.0
            .lock()
            .map(|tail| tail.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }
}

/// Reports each non-empty line of `pipe` as an output at `level`, keeping it in `tail` if given.
fn spawn_reader(
    pipe: impl std::io::Read + Send + 'static,
    level: Level,
    outputs: OutputSender,
    tail: Option<StderrTail>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            if !line.is_empty() {
                if let Some(tail) = &tail {
                    tail.push(&line);
                }
                outputs.send(ChannelOutput::new(level, line));
            }
        }
    })
}

/// Starts reader threads for `child`'s piped stdout (info) and stderr (error, kept in `tail`).
fn spawn_readers(
    child: &mut std::process::Child,
    outputs: &OutputSender,
    tail: &StderrTail,
) -> Vec<JoinHandle<()>> {
    let mut readers = vec![];
    if let Some(stdout) = child.stdout.take() {
        readers.push(spawn_reader(stdout, Level::Info, outputs.clone(), None));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(spawn_reader(
            stderr,
            Level::Error,
            outputs.clone(),
            Some(tail.clone()),
        ));
    }
    readers
}

/// Joins reader threads that finish within a short grace period and abandons the rest, such as
/// one stuck on a pipe an orphaned grandchild keeps open.
fn join_readers(readers: &mut Vec<JoinHandle<()>>) {
    let deadline = Instant::now() + READER_JOIN_GRACE;
    for handle in readers.drain(..) {
        while !handle.is_finished() && Instant::now() < deadline {
            thread::sleep(READER_POLL_INTERVAL);
        }
        if handle.is_finished() {
            let _ = handle.join();
        }
    }
}
