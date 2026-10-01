use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
use console::{Term, style};
use rngo::spec::Spec;
use rngo::{Input, Metadata, Output, RunLogWriter};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

const RENDER_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct ChannelStats {
    effects: u64,
    outputs: u64,
}

pub struct StatusWriter {
    child: Rc<dyn RunLogWriter>,
    effect_channels: Rc<HashMap<String, String>>,
    term: Term,
    stats: RefCell<BTreeMap<String, ChannelStats>>,
    last_timestamp: Cell<Option<DateTime<FixedOffset>>>,
    waiting: Cell<Option<DateTime<FixedOffset>>>,
    rendered_lines: Cell<usize>,
    last_render: Cell<Option<Instant>>,
    dirty: Cell<bool>,
}

impl StatusWriter {
    pub fn new<T: RunLogWriter + 'static>(child: Rc<T>, spec: &Spec) -> Rc<Self> {
        let effect_channels = spec
            .effects
            .iter()
            .filter_map(|(k, v)| v.channel.as_ref().map(|s| (k.clone(), s.clone())))
            .collect();

        Rc::new(StatusWriter {
            child: child as Rc<dyn RunLogWriter>,
            effect_channels: Rc::new(effect_channels),
            term: Term::stderr(),
            stats: RefCell::new(BTreeMap::new()),
            last_timestamp: Cell::new(None),
            waiting: Cell::new(None),
            rendered_lines: Cell::new(0),
            last_render: Cell::new(None),
            dirty: Cell::new(false),
        })
    }

    pub fn finish(&self) {
        self.waiting.set(None);
        self.render(true);
    }

    /// Shows that the run is waiting for an input due at `until`, or clears that state with `None`.
    pub fn wait(&self, until: Option<DateTime<FixedOffset>>) {
        if until.is_none() && self.waiting.get().is_none() {
            return;
        }
        self.waiting.set(until);
        self.dirty.set(true);
        self.render(false);
    }

    fn render(&self, force: bool) {
        if !self.term.is_term() {
            return;
        }

        if force && !self.dirty.get() {
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
        self.dirty.set(false);

        let time_line = match (self.waiting.get(), self.last_timestamp.get()) {
            (Some(until), _) => format!(
                "waiting: next input at {} (in {})",
                until.format("%Y-%m-%d %H:%M:%S"),
                format_remaining(until.to_utc() - Utc::now())
            ),
            (None, Some(timestamp)) => {
                format!("time: {}", timestamp.format("%Y-%m-%d %H:%M:%S"))
            }
            (None, None) => "time: -".to_string(),
        };

        let stats = self.stats.borrow();
        let mut lines = Vec::with_capacity(stats.len() + 2);
        lines.push(style("Simulation").bold().for_stderr().to_string());
        lines.push(time_line);
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

fn format_remaining(remaining: TimeDelta) -> String {
    let secs = remaining.num_seconds().max(0);
    let (h, m, s) = (secs / 3_600, secs % 3_600 / 60, secs % 60);
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

impl std::fmt::Debug for StatusWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusWriter").finish_non_exhaustive()
    }
}

impl RunLogWriter for StatusWriter {
    fn push_input(&self, input: Input) {
        self.last_timestamp.set(Some(input.timestamp));
        if let Some(channel) = self.effect_channels.get(&input.effect) {
            self.stats
                .borrow_mut()
                .entry(channel.clone())
                .or_default()
                .effects += 1;
        }

        self.dirty.set(true);
        self.render(false);
        self.child.push_input(input);
    }

    fn push_output(&self, output: Output) {
        self.stats
            .borrow_mut()
            .entry(output.channel.clone())
            .or_default()
            .outputs += 1;

        self.dirty.set(true);
        self.render(false);
        self.child.push_output(output);
    }

    fn push_metadata(&self, metadata: Metadata) {
        self.render(false);
        self.child.push_metadata(metadata);
    }
}

impl Drop for StatusWriter {
    fn drop(&mut self) {
        self.finish();
    }
}
