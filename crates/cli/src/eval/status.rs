use console::{Term, style};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use rngo::{Input, Metadata, Output, RunLogError, RunLogWriter};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

const RENDER_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct ChannelStats {
    inputs: u64,
    outputs: u64,
}

pub struct StatusWriter {
    child: Arc<dyn RunLogWriter>,
    effect_channels: HashMap<String, String>,
    term: Term,
    display: Mutex<Display>,
}

#[derive(Default)]
struct Display {
    stats: BTreeMap<String, ChannelStats>,
    last_timestamp: Option<Timestamp>,
    waiting: Option<Timestamp>,
    rendered_lines: usize,
    last_render: Option<Instant>,
    dirty: bool,
}

impl StatusWriter {
    /// `effect_channels` maps each effect key to the channel it writes to.
    pub fn new<T: RunLogWriter + 'static>(
        child: Arc<T>,
        effect_channels: HashMap<String, String>,
    ) -> Self {
        StatusWriter {
            child: child as Arc<dyn RunLogWriter>,
            effect_channels,
            term: Term::stderr(),
            display: Mutex::new(Display::default()),
        }
    }

    fn display(&self) -> MutexGuard<'_, Display> {
        self.display.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn finish(&self) {
        let mut display = self.display();
        display.waiting = None;
        self.render(&mut display, true);
    }

    /// Shows that the eval is waiting for an input due at `until`, until that time passes.
    pub fn wait(&self, until: Timestamp) {
        let mut display = self.display();
        display.waiting = Some(until);
        display.dirty = true;
        self.render(&mut display, false);
    }

    fn render(&self, display: &mut Display, force: bool) {
        if !self.term.is_term() {
            return;
        }

        if force && !display.dirty {
            return;
        }

        let now = Instant::now();
        if !force
            && let Some(last) = display.last_render
            && now.duration_since(last) < RENDER_INTERVAL
        {
            return;
        }
        display.last_render = Some(now);
        display.dirty = false;

        let waiting = display.waiting.filter(|until| *until > Timestamp::now());
        let time_line = match (waiting, display.last_timestamp) {
            (Some(until), _) => format!(
                "waiting: next input at {} (in {})",
                format_time(until),
                format_remaining(Timestamp::now().duration_until(until))
            ),
            (None, Some(timestamp)) => {
                format!("time: {}", format_time(timestamp))
            }
            (None, None) => "time: -".to_string(),
        };

        let mut lines = Vec::with_capacity(display.stats.len() + 2);
        lines.push(style("Simulation").bold().for_stderr().to_string());
        lines.push(time_line);
        for (channel, channel_stats) in display.stats.iter() {
            lines.push(channel_line(
                channel,
                channel_stats.inputs,
                channel_stats.outputs,
            ));
        }

        let _ = self.term.clear_last_lines(display.rendered_lines);
        for line in &lines {
            let _ = self.term.write_line(line);
        }
        display.rendered_lines = lines.len();
    }
}

fn count(n: u64, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn channel_line(channel: &str, inputs: u64, outputs: u64) -> String {
    format!(
        "{channel}: {}, {}",
        count(inputs, "input"),
        count(outputs, "output")
    )
}

fn format_time(timestamp: Timestamp) -> String {
    timestamp
        .to_zoned(TimeZone::UTC)
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn format_remaining(remaining: SignedDuration) -> String {
    let secs = remaining.as_secs().max(0);
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
    fn push_input(&self, input: Input) -> Result<(), RunLogError> {
        {
            let mut display = self.display();
            display.last_timestamp = Some(input.timestamp);
            if let Some(channel) = self.effect_channels.get(&input.effect) {
                display.stats.entry(channel.clone()).or_default().inputs += 1;
            }
            display.dirty = true;
            self.render(&mut display, false);
        }
        self.child.push_input(input)
    }

    fn push_output(&self, output: Output) -> Result<(), RunLogError> {
        {
            let mut display = self.display();
            display
                .stats
                .entry(output.channel.clone())
                .or_default()
                .outputs += 1;
            display.dirty = true;
            self.render(&mut display, false);
        }
        self.child.push_output(output)
    }

    fn push_metadata(&self, metadata: Metadata) -> Result<(), RunLogError> {
        self.render(&mut self.display(), false);
        self.child.push_metadata(metadata)
    }

    fn flush(&self) -> Result<(), RunLogError> {
        self.child.flush()
    }
}

impl Drop for StatusWriter {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_line_pluralizes_counts() {
        assert_eq!(channel_line("api", 0, 0), "api: 0 inputs, 0 outputs");
        assert_eq!(channel_line("api", 1, 1), "api: 1 input, 1 output");
        assert_eq!(channel_line("db", 31270, 2), "db: 31270 inputs, 2 outputs");
    }
}
