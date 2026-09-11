use chrono::{DateTime, FixedOffset};
use console::{Term, style};
use rngo_sim::spec::Spec;
use rngo_sim::{EffectMetadata, Input, Output, RunLog, RunLogReader, RunLogWriter};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Minimum real time between redraws, so a fast-running simulation doesn't spend its time
/// repainting the terminal instead of processing events.
const RENDER_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct ChannelStats {
    effects: u64,
    outputs: u64,
}

/// A [`RunLog`] proxy that, on [`RunLog::writer`], hands out a [`RunLogWriter`] that renders a
/// live-updating status block to stderr - the current simulated time and, per channel, how many
/// effects and outputs it has produced - leaving stdout free for `--stdout` event output. Forwards
/// every event to `child`'s own writer unchanged.
pub struct StatusRunLog {
    child: Box<dyn RunLog>,
    effect_channels: Rc<HashMap<String, String>>,
}

impl StatusRunLog {
    pub fn new(child: Box<dyn RunLog>, spec: &Spec) -> Self {
        let effect_channels = spec
            .effects
            .iter()
            .filter_map(|(k, v)| v.channel.as_ref().map(|s| (k.clone(), s.clone())))
            .collect();

        StatusRunLog {
            child,
            effect_channels: Rc::new(effect_channels),
        }
    }
}

impl std::fmt::Debug for StatusRunLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusRunLog").finish_non_exhaustive()
    }
}

impl RunLog for StatusRunLog {
    fn reader(&self) -> Rc<dyn RunLogReader> {
        self.child.reader()
    }

    fn writer(&self) -> Rc<dyn RunLogWriter> {
        Rc::new(StatusRunLogWriter {
            child: self.child.writer(),
            effect_channels: Rc::clone(&self.effect_channels),
            term: Term::stderr(),
            stats: RefCell::new(BTreeMap::new()),
            last_timestamp: Cell::new(None),
            rendered_lines: Cell::new(0),
            last_render: Cell::new(None),
        })
    }
}

struct StatusRunLogWriter {
    child: Rc<dyn RunLogWriter>,
    effect_channels: Rc<HashMap<String, String>>,
    term: Term,
    stats: RefCell<BTreeMap<String, ChannelStats>>,
    last_timestamp: Cell<Option<DateTime<FixedOffset>>>,
    rendered_lines: Cell<usize>,
    last_render: Cell<Option<Instant>>,
}

impl StatusRunLogWriter {
    fn render(&self, force: bool) {
        if !self.term.is_term() {
            return;
        }

        let now = Instant::now();
        if !force
            && let Some(last) = self.last_render.get()
            && now.duration_since(last) < RENDER_INTERVAL
        {
            return;
        }
        self.last_render.set(Some(now));

        let time = match self.last_timestamp.get() {
            Some(timestamp) => timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
            None => "-".to_string(),
        };

        let stats = self.stats.borrow();
        let mut lines = Vec::with_capacity(stats.len() + 2);
        lines.push(style("Simulation").bold().for_stderr().to_string());
        lines.push(format!("time: {time}"));
        for (channel, channel_stats) in stats.iter() {
            lines.push(format!(
                "{channel}: {} effects, {} outputs",
                channel_stats.effects, channel_stats.outputs
            ));
        }
        drop(stats);

        let _ = self.term.clear_last_lines(self.rendered_lines.get());
        for line in &lines {
            let _ = self.term.write_line(line);
        }
        self.rendered_lines.set(lines.len());
    }
}

impl std::fmt::Debug for StatusRunLogWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusRunLogWriter").finish_non_exhaustive()
    }
}

impl RunLogWriter for StatusRunLogWriter {
    fn push_input(&self, input: Input) {
        self.last_timestamp.set(Some(input.timestamp));
        if let Some(channel) = self.effect_channels.get(&input.effect) {
            self.stats
                .borrow_mut()
                .entry(channel.clone())
                .or_default()
                .effects += 1;
        }

        self.render(false);
        self.child.push_input(input);
    }

    fn push_output(&self, output: Output) {
        self.stats
            .borrow_mut()
            .entry(output.channel.clone())
            .or_default()
            .outputs += 1;

        self.render(false);
        self.child.push_output(output);
    }

    fn push_metadata(&self, metadata: EffectMetadata) {
        self.render(false);
        self.child.push_metadata(metadata);
    }

    fn get_signal_value(&self, query: &str) -> Option<serde_json::Value> {
        self.child.get_signal_value(query)
    }
}

impl Drop for StatusRunLogWriter {
    fn drop(&mut self) {
        // Guarantees the block reflects final counts even if the last update landed inside the
        // render throttle window.
        self.render(true);
    }
}
