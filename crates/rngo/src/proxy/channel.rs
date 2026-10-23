pub(crate) mod exec;
pub(crate) mod stdout;
pub(crate) mod stream;

pub use exec::Exec;
pub use stdout::Stdout;
pub use stream::Stream;

use crate::effect::Input;
use crate::proxy::format::Format;
use crate::proxy::output::{ChannelOutput, OutputSender};
use crate::{BuildError, ChannelError, Level};
use serde_json::Value;
use std::process::Command;

/// Where a proxy delivers the inputs of the effects assigned to it.
pub trait Channel: std::fmt::Debug {
    /// Sends one input, formatted by the channel's own format if it has one. The proxy ties
    /// returned outputs to the input.
    fn send(&mut self, input: &Input) -> Result<Vec<ChannelOutput>, ChannelError>;
    fn finish(&mut self) {}
}

/// Built with the channel's own key passed in at build time, rather than baked in at
/// construction - lets a builder (e.g. [`crate::build::exec`]) be constructed generically,
/// before the key it'll be added under is known, mirroring how `outputs` is threaded in.
pub trait ChannelBuilder {
    fn build(
        &self,
        channel_key: &str,
        outputs: OutputSender,
    ) -> Result<Box<dyn Channel>, Vec<BuildError>>;
}

/// The value a channel delivers for `input`: its formatted string, or its own `data` without a
/// format. A format failure comes back as the error output to report instead.
pub(crate) fn render(format: Option<&dyn Format>, input: &Input) -> Result<Value, ChannelOutput> {
    match format.map(|f| f.format(input)) {
        Some(Ok(data)) => Ok(Value::String(data)),
        Some(Err(error)) => Err(ChannelOutput::new(
            Level::Error,
            format!("format failed: {error}"),
        )),
        None => Ok(input.data.clone()),
    }
}

/// Builds an `sh -c <command>` process in its own process group, so a terminal Ctrl-C
/// reaches rngo but not its channel subprocesses.
fn shell(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd
}
