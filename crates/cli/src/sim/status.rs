use chrono::{DateTime, FixedOffset};
use console::{Term, style};
use rngo_sim::{Log, LogEvent, LogReader};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Minimum real time between redraws, so a fast-running simulation doesn't spend its time
/// repainting the terminal instead of processing events.
const RENDER_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct SystemStats {
    effects: u64,
    signals: u64,
}

/// A [`Log`] proxy that renders a live-updating status block to stderr - the current simulated
/// time and, per system, how many effects and signals it has produced - leaving stdout free for
/// `--stdout` event output. Forwards every event to `child` unchanged.
pub struct StatusLog {
    child: Box<dyn Log>,
    effect_systems: HashMap<String, String>,
    term: Term,
    stats: BTreeMap<String, SystemStats>,
    last_timestamp: Option<DateTime<FixedOffset>>,
    rendered_lines: usize,
    last_render: Option<Instant>,
}

impl StatusLog {
    pub fn new(child: Box<dyn Log>, effect_systems: HashMap<String, String>) -> Self {
        StatusLog {
            child,
            effect_systems,
            term: Term::stderr(),
            stats: BTreeMap::new(),
            last_timestamp: None,
            rendered_lines: 0,
            last_render: None,
        }
    }

    fn render(&mut self, force: bool) {
        if !self.term.is_term() {
            return;
        }

        let now = Instant::now();
        if !force
            && let Some(last) = self.last_render
            && now.duration_since(last) < RENDER_INTERVAL
        {
            return;
        }
        self.last_render = Some(now);

        let time = match self.last_timestamp {
            Some(timestamp) => timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
            None => "-".to_string(),
        };

        let mut lines = Vec::with_capacity(self.stats.len() + 2);
        lines.push(style("Simulation").bold().for_stderr().to_string());
        lines.push(format!("time: {time}"));
        for (system, stats) in &self.stats {
            lines.push(format!(
                "{system}: {} effects, {} signals",
                stats.effects, stats.signals
            ));
        }

        let _ = self.term.clear_last_lines(self.rendered_lines);
        for line in &lines {
            let _ = self.term.write_line(line);
        }
        self.rendered_lines = lines.len();
    }
}

impl std::fmt::Debug for StatusLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusLog").finish_non_exhaustive()
    }
}

impl Log for StatusLog {
    fn push(&mut self, event: LogEvent) {
        match &event {
            LogEvent::Effect(e) => {
                self.last_timestamp = Some(e.timestamp);
                if let Some(system) = self.effect_systems.get(&e.key) {
                    self.stats.entry(system.clone()).or_default().effects += 1;
                }
            }
            LogEvent::Signal(s) => {
                self.stats.entry(s.system.clone()).or_default().signals += 1;
            }
            LogEvent::Error(_) => {}
        }

        self.render(false);
        self.child.push(event);
    }

    fn reader(&self) -> Rc<dyn LogReader> {
        self.child.reader()
    }
}

impl Drop for StatusLog {
    fn drop(&mut self) {
        // Guarantees the block reflects final counts even if the last update landed inside the
        // render throttle window.
        self.render(true);
    }
}
