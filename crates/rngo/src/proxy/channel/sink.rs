pub(crate) mod exec;
pub(crate) mod stdout;
pub(crate) mod stream;

pub use exec::Exec;
pub use stdout::Stdout;
pub use stream::Stream;

use crate::Level;
use crate::effect::Input;
use crate::proxy::format::Format;
use crate::proxy::output::SinkOutput;
use serde_json::Value;
use std::process::Command;

/// The value a sink delivers for `input`: its formatted string, or its own `data` without a
/// format. A format failure comes back as the error output to report instead.
pub(crate) fn render(format: Option<&dyn Format>, input: &Input) -> Result<Value, SinkOutput> {
    match format.map(|f| f.format(input)) {
        Some(Ok(data)) => Ok(Value::String(data)),
        Some(Err(message)) => Err(SinkOutput::new(
            Level::Error,
            format!("format failed: {message}"),
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
