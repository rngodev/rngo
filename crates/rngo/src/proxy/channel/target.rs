pub(crate) mod exec;
pub(crate) mod stdout;
pub(crate) mod stream;

pub use exec::Exec;
pub use stdout::Stdout;
pub use stream::Stream;

use std::process::Command;

/// Builds an `sh -c <command>` process in its own process group, so a terminal Ctrl-C
/// reaches rngo but not its channel subprocesses.
fn shell(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd
}
