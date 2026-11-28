use crate::effect::Input;
use crate::error::{BuildErrors, ParseErrors};
use crate::proxy::channel::{
    Channel, ChannelBuildContext, ChannelBuilder, ChannelParseContext, ChannelParser, SendContext,
    StderrTail,
};
use crate::proxy::format::{Format, FormatBuilder};
use crate::proxy::output::{ChannelOutput, OutputSender};
use crate::{BuildError, ChannelError, Level, ParseError, spec};
use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// The most inputs a batch holds when the channel sets no `size`.
pub(crate) const DEFAULT_SIZE: usize = 10_000;

/// The environment variable that holds the path of the batch file a command loads.
const BATCH_FILE_VAR: &str = "RNGO_BATCH_FILE";

#[derive(Debug)]
pub(crate) struct Batch {
    channel_key: String,
    format: Option<Arc<dyn Format>>,
    command: String,
    size: usize,
    dir: PathBuf,
    outputs: OutputSender,
    next_number: u64,
    open: Option<OpenBatch>,
}

#[derive(Debug)]
struct OpenBatch {
    file: File,
    path: PathBuf,
    inputs: usize,
}

impl Batch {
    pub(crate) fn parser() -> BatchParser {
        BatchParser {}
    }

    pub(crate) fn builder() -> BatchBuilder {
        BatchBuilder::default()
    }

    fn write_error(&self, source: std::io::Error) -> ChannelError {
        ChannelError::write(self.channel_key.clone(), source)
    }

    /// The open batch, starting a new file if none is open.
    fn open_batch(&mut self) -> Result<&mut OpenBatch, ChannelError> {
        if self.open.is_none() {
            fs::create_dir_all(&self.dir).map_err(|e| self.write_error(e))?;
            self.next_number += 1;
            let path = self.dir.join(format!("{:08}", self.next_number));
            let file = File::create(&path).map_err(|e| self.write_error(e))?;
            self.open = Some(OpenBatch {
                file,
                path,
                inputs: 0,
            });
        }
        Ok(self.open.as_mut().expect("a batch was just opened"))
    }

    /// Runs the command on the open batch, if any; deletes its file once the command succeeds.
    fn deliver(&mut self) -> Result<(), ChannelError> {
        let Some(OpenBatch { file, path, inputs }) = self.open.take() else {
            return Ok(());
        };
        drop(file);

        let stdin = File::open(&path).map_err(|e| self.write_error(e))?;
        let mut child = super::shell(&self.command)
            .env(BATCH_FILE_VAR, &path)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| ChannelError::spawn(self.channel_key.clone(), source))?;

        let tail = StderrTail::default();
        let mut readers = super::spawn_readers(&mut child, &self.outputs, &tail);
        let status = child.wait();
        super::join_readers(&mut readers);
        let status = status.map_err(|e| self.write_error(e))?;

        if !status.success() {
            return Err(ChannelError::batch(
                self.channel_key.clone(),
                status.to_string(),
                inputs,
                tail.text(),
                path,
            ));
        }

        if let Err(error) = fs::remove_file(&path) {
            self.outputs.send(ChannelOutput::new(
                Level::Warning,
                format!("failed to delete batch file {}: {error}", path.display()),
            ));
        }
        Ok(())
    }
}

impl Channel for Batch {
    fn send(
        &mut self,
        input: &Input,
        context: &SendContext,
    ) -> Result<Vec<ChannelOutput>, ChannelError> {
        let data = match super::render(self.format.as_deref(), input, context) {
            Ok(data) => data,
            Err(output) => return Ok(vec![output]),
        };
        let line = match data {
            Value::String(s) => s + "\n",
            other => format!("{other}\n"),
        };

        let batch = self.open_batch()?;
        let written = batch
            .file
            .write_all(line.as_bytes())
            .and_then(|()| batch.file.flush());
        batch.inputs += 1;
        let full = batch.inputs >= self.size;
        written.map_err(|e| self.write_error(e))?;

        if full {
            self.deliver()?;
        }
        Ok(vec![])
    }

    fn finish(&mut self) -> Result<(), ChannelError> {
        self.deliver()
    }
}

/// Builds a `batch` channel, which appends inputs to a file and loads each full file with its
/// command.
#[derive(Default)]
pub struct BatchBuilder {
    command: Option<String>,
    size: Option<usize>,
    format: Option<Box<dyn FormatBuilder>>,
}

impl BatchBuilder {
    pub fn format(mut self, format: impl FormatBuilder + 'static) -> Self {
        self.format = Some(Box::new(format));
        self
    }

    /// The shell command run on each batch. It gets the batch file's path in `RNGO_BATCH_FILE`
    /// and its contents on stdin.
    pub fn command(mut self, value: impl Into<String>) -> Self {
        self.command = Some(value.into());
        self
    }

    /// The most inputs a batch holds before it is sent (10,000 by default). Ignored in realtime,
    /// where each input is sent on its own.
    pub fn size(mut self, size: usize) -> Self {
        self.size = Some(size);
        self
    }
}

impl ChannelBuilder for BatchBuilder {
    fn build(
        self: Box<Self>,
        context: ChannelBuildContext,
    ) -> Result<Box<dyn Channel>, BuildErrors> {
        let channel_key = context.key();
        let Some(command) = self.command else {
            return Err(BuildError::channel(channel_key, "command not specified").into());
        };
        let size = self.size.unwrap_or(DEFAULT_SIZE);
        if size == 0 {
            return Err(BuildError::channel(channel_key, "size must be positive").into());
        }
        let format = super::build_format(self.format, &context)?;

        let base = match context.dir() {
            Some(dir) => dir.to_path_buf(),
            None => temp_dir(),
        };
        let dir = std::path::absolute(&base)
            .unwrap_or(base)
            .join("batches")
            .join(channel_key);

        Ok(Box::new(Batch {
            channel_key: channel_key.to_string(),
            format,
            command,
            size: if context.realtime() { 1 } else { size },
            dir,
            outputs: context.outputs(),
            next_number: 0,
            open: None,
        }))
    }
}

/// A new directory name under the system's temporary directory, for a proxy given none.
fn temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    Path::new(&std::env::temp_dir()).join(format!("rngo-{}-{nanos}", std::process::id()))
}

pub(crate) struct BatchParser {}

impl ChannelParser for BatchParser {
    fn key(&self) -> &str {
        "batch"
    }

    fn parse(
        &self,
        channel: &spec::Channel,
        context: &ChannelParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, ParseErrors> {
        let command = match channel.fields.get("command") {
            Some(Value::String(command)) => command,
            Some(_) => {
                return Err(ParseError::invalid("must be a string")
                    .at(["command"])
                    .into());
            }
            None => {
                return Err(ParseError::invalid("is required for a batch channel")
                    .at(["command"])
                    .into());
            }
        };

        let mut builder = BatchBuilder::default().command(command);

        if let Some(value) = channel.fields.get("size") {
            match value
                .as_u64()
                .filter(|size| *size >= 1)
                .and_then(|size| usize::try_from(size).ok())
            {
                Some(size) => builder = builder.size(size),
                None => {
                    return Err(ParseError::invalid("must be a positive integer")
                        .at(["size"])
                        .into());
                }
            }
        }

        if let Some(value) = channel.fields.get("format")
            && let Some(format) = context.parse_format(&["format"], value)?
        {
            builder = builder.format(format);
        }

        Ok(Box::new(builder))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::build;
    use crate::proxy::output::Output;
    use jiff::Timestamp;
    use serde_json::json;
    use std::sync::mpsc::{self, Receiver};
    use tempfile::TempDir;

    #[derive(Debug)]
    struct FailOnEvenIds;

    impl Format for FailOnEvenIds {
        fn format(
            &self,
            input: &Input,
            _context: &crate::format::FormatContext,
        ) -> Result<String, crate::FormatError> {
            if input.id.is_multiple_of(2) {
                Err(crate::FormatError::Render("boom".into()))
            } else {
                Ok(input.id.to_string())
            }
        }
    }

    fn input(id: u64, data: Value) -> Input {
        Input::new(id, "user", Timestamp::now(), data)
    }

    fn batch(
        builder: BatchBuilder,
        dir: &Path,
        realtime: bool,
    ) -> (Box<dyn Channel>, Receiver<Output>) {
        let (tx, rx) = mpsc::channel();
        let channel = Box::new(builder)
            .build(
                ChannelBuildContext::new("db", OutputSender::new("db", tx))
                    .with_dir(Some(dir))
                    .with_realtime(realtime),
            )
            .unwrap();
        (channel, rx)
    }

    fn send(channel: &mut Box<dyn Channel>, id: u64) -> Result<Vec<ChannelOutput>, ChannelError> {
        channel.send(&input(id, json!({ "id": id })), &SendContext::new())
    }

    fn batches(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(dir.join("batches/db")) else {
            return vec![];
        };
        let mut paths: Vec<_> = entries.map(|e| e.unwrap().path()).collect();
        paths.sort();
        paths
    }

    #[test]
    fn sends_a_batch_once_it_reaches_size_and_deletes_its_file() {
        let dir = TempDir::new().unwrap();
        let loaded = dir.path().join("loaded");
        let command = format!("cat >> {}", loaded.display());
        let (mut channel, _rx) = batch(build::batch().command(command).size(2), dir.path(), false);

        send(&mut channel, 1).unwrap();
        assert_eq!(batches(dir.path()).len(), 1);
        assert!(!loaded.exists());

        send(&mut channel, 2).unwrap();
        assert!(batches(dir.path()).is_empty());
        assert_eq!(
            fs::read_to_string(&loaded).unwrap(),
            "{\"id\":1}\n{\"id\":2}\n"
        );

        send(&mut channel, 3).unwrap();
        channel.finish().unwrap();
        assert!(batches(dir.path()).is_empty());
        assert!(
            fs::read_to_string(&loaded)
                .unwrap()
                .ends_with("{\"id\":3}\n")
        );
    }

    #[test]
    fn the_command_gets_the_file_path_in_an_environment_variable() {
        let dir = TempDir::new().unwrap();
        let loaded = dir.path().join("loaded");
        let command = format!("cp \"$RNGO_BATCH_FILE\" {}", loaded.display());
        let (mut channel, _rx) = batch(build::batch().command(command), dir.path(), false);

        send(&mut channel, 1).unwrap();
        channel.finish().unwrap();
        assert_eq!(fs::read_to_string(&loaded).unwrap(), "{\"id\":1}\n");
    }

    #[test]
    fn a_failed_batch_keeps_its_file_and_says_why() {
        let dir = TempDir::new().unwrap();
        let (mut channel, _rx) = batch(
            build::batch().command("cat > /dev/null; echo 'no such table' >&2; exit 3"),
            dir.path(),
            false,
        );

        send(&mut channel, 1).unwrap();
        send(&mut channel, 2).unwrap();
        let error = channel.finish().unwrap_err();

        let kept = batches(dir.path());
        assert_eq!(kept.len(), 1);
        let message = error.to_string();
        assert!(message.contains("channel `db`"), "{message}");
        assert!(message.contains("exit status: 3"), "{message}");
        assert!(message.contains("batch of 2 inputs"), "{message}");
        assert!(message.contains("no such table"), "{message}");
        assert!(
            message.contains(&kept[0].display().to_string()),
            "{message}"
        );
    }

    #[test]
    fn realtime_sends_each_input_on_its_own() {
        let dir = TempDir::new().unwrap();
        let loaded = dir.path().join("loaded");
        let command = format!("cat >> {}; echo loaded", loaded.display());
        let (mut channel, rx) = batch(build::batch().command(command).size(10), dir.path(), true);

        send(&mut channel, 1).unwrap();
        assert_eq!(fs::read_to_string(&loaded).unwrap(), "{\"id\":1}\n");
        assert!(batches(dir.path()).is_empty());

        let output = rx.try_recv().unwrap();
        assert_eq!(output.data, "loaded");
        assert_eq!(output.input_id, None);
    }

    #[test]
    fn a_format_failure_leaves_the_input_out_of_its_batch() {
        let dir = TempDir::new().unwrap();
        let loaded = dir.path().join("loaded");
        let command = format!("cat >> {}", loaded.display());
        let (mut channel, _rx) = batch(
            build::batch().command(command).format(FailOnEvenIds),
            dir.path(),
            false,
        );

        assert!(send(&mut channel, 1).unwrap().is_empty());
        let outputs = send(&mut channel, 2).unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].level, Level::Error);
        assert!(send(&mut channel, 3).unwrap().is_empty());

        channel.finish().unwrap();
        assert_eq!(fs::read_to_string(&loaded).unwrap(), "1\n3\n");
    }

    #[test]
    fn works_with_the_sql_format() {
        let dir = TempDir::new().unwrap();
        let loaded = dir.path().join("loaded");
        let command = format!("cat >> {}", loaded.display());
        let (mut channel, _rx) = batch(
            build::batch()
                .command(command)
                .format(build::sql_format().effect_table("user", "users")),
            dir.path(),
            false,
        );

        channel
            .send(&input(1, json!({ "name": "ann" })), &SendContext::new())
            .unwrap();
        channel.finish().unwrap();
        let loaded = fs::read_to_string(&loaded).unwrap();
        assert!(loaded.starts_with("INSERT INTO users"), "{loaded}");
    }

    #[test]
    fn a_zero_size_is_a_build_error() {
        let (tx, _rx) = mpsc::channel();
        let result = Box::new(build::batch().command("cat").size(0))
            .build(ChannelBuildContext::new("db", OutputSender::new("db", tx)));
        assert!(result.is_err());
    }

    #[test]
    fn the_parser_rejects_a_size_that_isnt_a_positive_integer() {
        let spec = crate::spec::from_value(json!({ "effects": {} })).unwrap();
        let context = ChannelParseContext::new(&[], &spec);
        for size in [json!(0), json!(-1), json!(1.5), json!("10")] {
            let channel: spec::Channel =
                serde_json::from_value(json!({ "type": "batch", "command": "cat", "size": size }))
                    .unwrap();
            let errors = Batch::parser().parse(&channel, &context).err().unwrap();
            assert_eq!(errors[0].path(), ["size".to_string()], "{size}");
        }
    }
}
