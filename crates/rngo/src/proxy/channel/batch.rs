use crate::effect::Input;
use crate::error::{BuildErrors, ParseErrors};
use crate::proxy::channel::{
    Channel, ChannelBuildContext, ChannelBuilder, ChannelParseContext, ChannelParser, SendContext,
    StderrTail,
};
use crate::proxy::format::{Format, FormatBuilder};
use crate::proxy::output::{ChannelOutput, OutputSender};
use crate::{BatchTarget, BuildError, ChannelError, Level, ParseError, spec};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// The most inputs a batch holds when the channel sets no `size`.
pub(crate) const DEFAULT_SIZE: usize = 10_000;

/// The environment variable that holds the path of the batch file a command loads.
const BATCH_FILE_VAR: &str = "RNGO_BATCH_FILE";
/// The environment variable that holds the effect of a file split by effect.
const BATCH_EFFECT_VAR: &str = "RNGO_BATCH_EFFECT";
/// The environment variable that holds the table of a file split by effect.
const BATCH_TABLE_VAR: &str = "RNGO_BATCH_TABLE";
/// The environment variable that holds the comma-separated columns of a file split by effect.
const BATCH_COLUMNS_VAR: &str = "RNGO_BATCH_COLUMNS";

#[derive(Debug)]
pub(crate) struct Batch {
    channel_key: String,
    format: Option<Arc<dyn Format>>,
    command: String,
    size: usize,
    dir: PathBuf,
    outputs: OutputSender,
    split: Option<Split>,
    next_number: u64,
    open: Option<OpenBatch>,
}

/// How a batch split by effect names and orders its files.
#[derive(Debug, Default)]
struct Split {
    tables: HashMap<String, String>,
    parents: BTreeMap<String, BTreeSet<String>>,
}

/// Which file of the open batch an input goes to: one for all inputs, or one per effect and
/// column list when split.
type FileKey = Option<(String, Vec<String>)>;

#[derive(Debug)]
struct OpenBatch {
    number: u64,
    files: Vec<BatchFile>,
    index: HashMap<FileKey, usize>,
    inputs: usize,
}

#[derive(Debug)]
struct BatchFile {
    file: File,
    path: PathBuf,
    effect: Option<String>,
    columns: Vec<String>,
    inputs: usize,
}

/// A written batch file waiting for its command.
#[derive(Debug)]
struct PendingFile {
    path: PathBuf,
    effect: Option<String>,
    columns: Vec<String>,
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

    /// The directory holding the files of batch `number` split by effect.
    fn split_dir(&self, number: u64) -> PathBuf {
        self.dir.join(format!("{number:08}"))
    }

    /// The index of the open batch's file for `input`, opening the batch and file as needed.
    fn file_for(&mut self, input: &Input) -> Result<usize, ChannelError> {
        let key: FileKey = self
            .split
            .as_ref()
            .map(|_| (input.effect.clone(), columns(&input.data)));

        if self.open.is_none() {
            fs::create_dir_all(&self.dir).map_err(|e| self.write_error(e))?;
            self.next_number += 1;
            self.open = Some(OpenBatch {
                number: self.next_number,
                files: vec![],
                index: HashMap::new(),
                inputs: 0,
            });
        }
        let number = self.open.as_ref().expect("a batch is open").number;
        if let Some(&index) = self.open.as_ref().and_then(|open| open.index.get(&key)) {
            return Ok(index);
        }

        let (file, path) = match &key {
            None => {
                let path = self.dir.join(format!("{number:08}"));
                let file = File::create(&path).map_err(|e| self.write_error(e))?;
                (file, path)
            }
            Some((effect, _)) => {
                let dir = self.split_dir(number);
                fs::create_dir_all(&dir).map_err(|e| self.write_error(e))?;
                create_numbered(&dir, &file_name(effect)).map_err(|e| self.write_error(e))?
            }
        };

        let open = self.open.as_mut().expect("a batch is open");
        let (effect, columns) = match key.clone() {
            Some((effect, columns)) => (Some(effect), columns),
            None => (None, vec![]),
        };
        open.files.push(BatchFile {
            file,
            path,
            effect,
            columns,
            inputs: 0,
        });
        let index = open.files.len() - 1;
        open.index.insert(key, index);
        Ok(index)
    }

    /// Runs the command on each file of the open batch, if any, parents first when split;
    /// deletes each file once its command succeeds and stops at the first that fails.
    fn deliver(&mut self) -> Result<(), ChannelError> {
        let Some(OpenBatch { number, files, .. }) = self.open.take() else {
            return Ok(());
        };
        let mut files: Vec<PendingFile> = files
            .into_iter()
            .map(|file| PendingFile {
                path: file.path,
                effect: file.effect,
                columns: file.columns,
                inputs: file.inputs,
            })
            .collect();

        if let Some(split) = &self.split {
            let order = load_order(
                &split.parents,
                files.iter().filter_map(|file| file.effect.as_deref()),
            );
            let rank: HashMap<&str, usize> = order
                .iter()
                .enumerate()
                .map(|(rank, effect)| (effect.as_str(), rank))
                .collect();
            files.sort_by_key(|file| file.effect.as_deref().map(|effect| rank[effect]));
        }

        for file in &files {
            self.load(file)?;
        }

        if self.split.is_some() {
            let dir = self.split_dir(number);
            if let Err(error) = fs::remove_dir(&dir) {
                self.warn(format!(
                    "failed to delete batch directory {}: {error}",
                    dir.display()
                ));
            }
        }
        Ok(())
    }

    /// Runs the command on `file`, then deletes it.
    fn load(&self, file: &PendingFile) -> Result<(), ChannelError> {
        let stdin = File::open(&file.path).map_err(|e| self.write_error(e))?;
        let mut command = super::shell(&self.command);
        command.env(BATCH_FILE_VAR, &file.path);
        let table = file.effect.as_deref().map(|effect| self.table(effect));
        if let (Some(effect), Some(table)) = (&file.effect, &table) {
            command
                .env(BATCH_EFFECT_VAR, effect)
                .env(BATCH_TABLE_VAR, table)
                .env(BATCH_COLUMNS_VAR, file.columns.join(","));
        }
        let mut child = command
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
            let error = ChannelError::batch(
                self.channel_key.clone(),
                status.to_string(),
                file.inputs,
                tail.text(),
                &file.path,
            );
            return Err(match (&file.effect, table) {
                (Some(effect), Some(table)) => {
                    error.with_batch_target(BatchTarget::new(effect, table))
                }
                _ => error,
            });
        }

        if let Err(error) = fs::remove_file(&file.path) {
            self.warn(format!(
                "failed to delete batch file {}: {error}",
                file.path.display()
            ));
        }
        Ok(())
    }

    /// The table `effect`'s files load into: its `metadata.table`, else its key.
    fn table(&self, effect: &str) -> String {
        self.split
            .as_ref()
            .and_then(|split| split.tables.get(effect))
            .map_or_else(|| effect.to_string(), String::clone)
    }

    fn warn(&self, message: String) {
        self.outputs
            .send(ChannelOutput::new(Level::Warning, message));
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

        let index = self.file_for(input)?;
        let open = self.open.as_mut().expect("a batch is open");
        let file = &mut open.files[index];
        let written = file
            .file
            .write_all(line.as_bytes())
            .and_then(|()| file.file.flush());
        file.inputs += 1;
        open.inputs += 1;
        let full = open.inputs >= self.size;
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

/// The top-level keys of `data`, the columns of its file; none if it isn't an object.
fn columns(data: &Value) -> Vec<String> {
    match data {
        Value::Object(map) => map.keys().cloned().collect(),
        _ => vec![],
    }
}

/// `effect` as a file name, with anything but ASCII letters, digits, `-` and `_` replaced by `_`.
fn file_name(effect: &str) -> String {
    let name: String = effect
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() { "_".into() } else { name }
}

/// Creates a new file named `name` in `dir`, or `name-1`, `name-2`, ... if taken.
fn create_numbered(dir: &Path, name: &str) -> std::io::Result<(File, PathBuf)> {
    let mut n = 0;
    loop {
        let path = if n == 0 {
            dir.join(name)
        } else {
            dir.join(format!("{name}-{n}"))
        };
        match File::create_new(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => n += 1,
            Err(error) => return Err(error),
        }
    }
}

/// `effects` and every effect in `parents`, each after its parents (`parents[effect]`), ties
/// in key order. Effects caught in a cycle, and those waiting on one, come last in key order.
fn load_order<'a>(
    parents: &BTreeMap<String, BTreeSet<String>>,
    effects: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut nodes: BTreeSet<&str> = effects.into_iter().collect();
    for (effect, effect_parents) in parents {
        nodes.insert(effect);
        nodes.extend(effect_parents.iter().map(String::as_str));
    }

    let mut waiting: BTreeMap<&str, usize> = BTreeMap::new();
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for &node in &nodes {
        let node_parents = parents.get(node).into_iter().flatten();
        let mut count = 0;
        for parent in node_parents.filter(|parent| parent.as_str() != node) {
            children.entry(parent).or_default().push(node);
            count += 1;
        }
        waiting.insert(node, count);
    }

    let mut ready: BTreeSet<&str> = waiting
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(node, _)| *node)
        .collect();
    let mut order = vec![];
    while let Some(node) = ready.pop_first() {
        order.push(node.to_string());
        waiting.remove(node);
        for &child in children.get(node).into_iter().flatten() {
            if let Some(count) = waiting.get_mut(child) {
                *count -= 1;
                if *count == 0 {
                    ready.insert(child);
                }
            }
        }
    }
    order.extend(waiting.keys().map(|node| node.to_string()));
    order
}

/// The effects each effect references, through `reference` generators (including those inside
/// named custom generator types) and effect triggers. An effect's reference to itself is left out.
pub(crate) fn effect_references(spec: &spec::Spec) -> BTreeMap<String, BTreeSet<String>> {
    let mut references = BTreeMap::new();
    for (key, effect) in &spec.effects {
        let mut parents = BTreeSet::new();
        let mut visited = BTreeSet::new();
        if let Ok(generator) = serde_json::to_value(&effect.generator) {
            collect_references(&generator, spec, &mut visited, &mut parents);
        }
        if let Some(spec::TriggerUnion::Full(spec::Trigger::Effect { key: parent })) =
            &effect.trigger
        {
            parents.insert(parent.clone());
        }
        parents.remove(key);
        if !parents.is_empty() {
            references.insert(key.clone(), parents);
        }
    }
    references
}

/// Adds the effects `value`'s `reference` generators name to `parents`, following custom
/// generator types not yet in `visited`.
fn collect_references(
    value: &Value,
    spec: &spec::Spec,
    visited: &mut BTreeSet<String>,
    parents: &mut BTreeSet<String>,
) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(kind)) = map.get("type") {
                if kind == "reference"
                    && let Some(Value::String(effect)) = map.get("effect")
                {
                    parents.insert(effect.clone());
                }
                if let Some(custom) = spec.generators.get(kind)
                    && visited.insert(kind.clone())
                    && let Ok(generator) = serde_json::to_value(&custom.generator)
                {
                    collect_references(&generator, spec, visited, parents);
                }
            }
            for value in map.values() {
                collect_references(value, spec, visited, parents);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_references(value, spec, visited, parents);
            }
        }
        _ => {}
    }
}

/// Builds a `batch` channel, which appends inputs to a file and loads each full file with its
/// command.
#[derive(Default)]
pub struct BatchBuilder {
    command: Option<String>,
    size: Option<usize>,
    format: Option<Box<dyn FormatBuilder>>,
    split: bool,
    tables: HashMap<String, String>,
    parents: BTreeMap<String, BTreeSet<String>>,
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

    /// The most inputs a batch holds before it is sent (10,000 by default), across all its files.
    /// Ignored in realtime, where each input is sent on its own.
    pub fn size(mut self, size: usize) -> Self {
        self.size = Some(size);
        self
    }

    /// Writes one file per effect and column list instead of one per batch, and runs the command
    /// once per file, parents first, with `RNGO_BATCH_EFFECT`, `RNGO_BATCH_TABLE` and
    /// `RNGO_BATCH_COLUMNS` set.
    pub fn split_by_effect(mut self) -> Self {
        self.split = true;
        self
    }

    /// Sets `RNGO_BATCH_TABLE` to `table` for `effect`'s files instead of the effect key. Only
    /// used with [`split_by_effect`](Self::split_by_effect).
    pub fn effect_table(mut self, effect: impl Into<String>, table: impl Into<String>) -> Self {
        self.tables.insert(effect.into(), table.into());
        self
    }

    /// Loads `effect`'s files after `parent`'s, such as when `effect` references `parent`. Only
    /// used with [`split_by_effect`](Self::split_by_effect).
    pub fn load_after(mut self, effect: impl Into<String>, parent: impl Into<String>) -> Self {
        let (effect, parent) = (effect.into(), parent.into());
        if effect != parent {
            self.parents.entry(effect).or_default().insert(parent);
        }
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
            split: self.split.then_some(Split {
                tables: self.tables,
                parents: self.parents,
            }),
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

        match channel.fields.get("split") {
            None => {}
            Some(Value::String(split)) if split == "effect" => {
                let spec = context.spec();
                builder = builder.split_by_effect();
                for (key, effect) in &spec.effects {
                    if let Some(table) = effect
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get("table"))
                        .and_then(Value::as_str)
                    {
                        builder = builder.effect_table(key, table);
                    }
                }
                for (effect, parents) in effect_references(spec) {
                    for parent in parents {
                        builder = builder.load_after(effect.clone(), parent);
                    }
                }
            }
            Some(_) => {
                return Err(ParseError::invalid("must be \"effect\"")
                    .at(["split"])
                    .into());
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

    fn send_to(
        channel: &mut Box<dyn Channel>,
        id: u64,
        effect: &str,
        data: Value,
    ) -> Result<Vec<ChannelOutput>, ChannelError> {
        channel.send(
            &Input::new(id, effect, Timestamp::now(), data),
            &SendContext::new(),
        )
    }

    /// Every file left under the channel's batch directory, relative to it.
    fn kept_files(dir: &Path) -> Vec<String> {
        let root = dir.join("batches/db");
        let mut files = vec![];
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path.strip_prefix(&root).unwrap().display().to_string());
                }
            }
        }
        files.sort();
        files
    }

    /// A command that appends a line naming each file's effect, table and columns, then the
    /// file, to `log`.
    fn logging_command(log: &Path) -> String {
        format!(
            "{{ echo \"$RNGO_BATCH_EFFECT $RNGO_BATCH_TABLE $RNGO_BATCH_COLUMNS\"; cat; }} >> {}",
            log.display()
        )
    }

    #[test]
    fn split_writes_one_file_per_effect_and_column_list() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("log");
        let (mut channel, _rx) = batch(
            build::batch()
                .command(logging_command(&log))
                .split_by_effect()
                .effect_table("user", "users"),
            dir.path(),
            false,
        );

        send_to(&mut channel, 1, "user", json!({ "id": 1, "name": "a" })).unwrap();
        send_to(&mut channel, 2, "user", json!({ "id": 2 })).unwrap();
        send_to(&mut channel, 3, "user", json!({ "id": 3, "name": "c" })).unwrap();
        send_to(&mut channel, 4, "event", json!({ "kind": "x" })).unwrap();
        assert_eq!(
            kept_files(dir.path()),
            ["00000001/event", "00000001/user", "00000001/user-1"]
        );

        channel.finish().unwrap();
        assert!(kept_files(dir.path()).is_empty());
        assert!(!dir.path().join("batches/db/00000001").exists());
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            "event event kind\n{\"kind\":\"x\"}\n\
             user users id,name\n{\"id\":1,\"name\":\"a\"}\n{\"id\":3,\"name\":\"c\"}\n\
             user users id\n{\"id\":2}\n"
        );
    }

    #[test]
    fn split_loads_parents_first() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("log");
        let command = format!("echo \"$RNGO_BATCH_EFFECT\" >> {}", log.display());
        let (mut channel, _rx) = batch(
            build::batch()
                .command(command)
                .split_by_effect()
                .load_after("comment", "post")
                .load_after("comment", "comment")
                .load_after("post", "author"),
            dir.path(),
            false,
        );

        for (id, effect) in [(1, "comment"), (2, "post"), (3, "author"), (4, "audit")] {
            send_to(&mut channel, id, effect, json!({ "id": id })).unwrap();
        }
        channel.finish().unwrap();
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            "audit\nauthor\npost\ncomment\n"
        );
    }

    #[test]
    fn a_failed_split_file_keeps_it_and_the_files_after_it() {
        let dir = TempDir::new().unwrap();
        let command = "cat > /dev/null; if [ \"$RNGO_BATCH_EFFECT\" = post ]; then echo 'bad fk' >&2; exit 1; fi";
        let (mut channel, _rx) = batch(
            build::batch()
                .command(command)
                .split_by_effect()
                .effect_table("post", "posts")
                .load_after("post", "author")
                .load_after("comment", "post"),
            dir.path(),
            false,
        );

        for (id, effect) in [(1, "comment"), (2, "post"), (3, "author")] {
            send_to(&mut channel, id, effect, json!({ "id": id })).unwrap();
        }
        let error = channel.finish().unwrap_err();

        assert_eq!(
            kept_files(dir.path()),
            ["00000001/comment", "00000001/post"]
        );
        let message = error.to_string();
        assert!(message.contains("channel `db`"), "{message}");
        assert!(message.contains("effect `post`"), "{message}");
        assert!(message.contains("table `posts`"), "{message}");
        assert!(message.contains("exit status: 1"), "{message}");
        assert!(message.contains("bad fk"), "{message}");
        assert!(message.contains("00000001/post"), "{message}");
    }

    #[test]
    fn split_size_counts_inputs_across_files() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("log");
        let command = format!("echo \"$RNGO_BATCH_FILE\" >> {}", log.display());
        let (mut channel, _rx) = batch(
            build::batch().command(command).split_by_effect().size(2),
            dir.path(),
            false,
        );

        send_to(&mut channel, 1, "a", json!({ "id": 1 })).unwrap();
        send_to(&mut channel, 2, "b", json!({ "id": 2 })).unwrap();
        assert!(kept_files(dir.path()).is_empty());
        send_to(&mut channel, 3, "a", json!({ "id": 3 })).unwrap();
        channel.finish().unwrap();

        let loaded = fs::read_to_string(&log).unwrap();
        let loaded: Vec<_> = loaded
            .lines()
            .map(|line| &line[line.len() - 10..])
            .collect();
        assert_eq!(loaded, ["00000001/a", "00000001/b", "00000002/a"]);
    }

    #[test]
    fn split_works_with_the_csv_format_in_realtime() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("log");
        let (mut channel, _rx) = batch(
            build::batch()
                .command(logging_command(&log))
                .split_by_effect()
                .format(build::csv_format()),
            dir.path(),
            true,
        );

        send_to(&mut channel, 1, "user", json!({ "id": 1, "name": "a" })).unwrap();
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            "user user id,name\n1,\"a\"\n"
        );
        assert!(kept_files(dir.path()).is_empty());
    }

    #[test]
    fn split_file_names_are_safe() {
        assert_eq!(file_name("a/b.c"), "a_b_c");
        assert_eq!(file_name(".."), "__");
        assert_eq!(file_name(""), "_");
    }

    #[test]
    fn load_order_puts_cycles_last() {
        let parents = BTreeMap::from([
            ("a".to_string(), BTreeSet::from(["b".to_string()])),
            ("b".to_string(), BTreeSet::from(["a".to_string()])),
            ("c".to_string(), BTreeSet::from(["a".to_string()])),
        ]);
        assert_eq!(load_order(&parents, ["d", "e"]), ["d", "e", "a", "b", "c"]);
    }

    #[test]
    fn effect_references_follow_reference_generators_custom_types_and_triggers() {
        let spec = crate::spec::from_value(json!({
            "generators": {
                "author_id": { "generator": { "type": "reference", "effect": "author" } },
                "loop": { "generator": { "type": "array", "items": { "type": "loop" } } }
            },
            "effects": {
                "author": { "generator": { "type": "loop" } },
                "post": {
                    "generator": {
                        "type": "object",
                        "properties": {
                            "author": { "type": "author_id" },
                            "parent": { "type": "reference", "effect": "post" }
                        }
                    }
                },
                "view": {
                    "trigger": { "type": "effect", "key": "post" },
                    "generator": { "type": "constant", "value": 1 }
                }
            }
        }))
        .unwrap();

        assert_eq!(
            effect_references(&spec),
            BTreeMap::from([
                ("post".to_string(), BTreeSet::from(["author".to_string()])),
                ("view".to_string(), BTreeSet::from(["post".to_string()])),
            ])
        );
    }

    #[test]
    fn the_parser_reads_split_tables_and_references() {
        let spec = crate::spec::from_value(json!({
            "effects": {
                "user": { "metadata": { "table": "users" }, "generator": { "type": "constant", "value": 1 } }
            }
        }))
        .unwrap();
        let context = ChannelParseContext::new(&[], &spec);
        let channel: spec::Channel =
            serde_json::from_value(json!({ "type": "batch", "command": "cat", "split": "effect" }))
                .unwrap();
        assert!(Batch::parser().parse(&channel, &context).is_ok());

        for split in [json!("table"), json!(true)] {
            let channel: spec::Channel = serde_json::from_value(
                json!({ "type": "batch", "command": "cat", "split": split }),
            )
            .unwrap();
            let errors = Batch::parser().parse(&channel, &context).err().unwrap();
            assert_eq!(errors[0].path(), ["split".to_string()], "{split}");
        }
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
