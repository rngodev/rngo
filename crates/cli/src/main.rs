mod init;
mod sim;

use clap::{Parser, Subcommand};

/// Simulate code usage, record everything and analyze the results
#[derive(Parser)]
#[command(name = "rngo", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a directory for rngo
    ///
    /// Creates a `.rngo` directory, a starter `.rngo/spec.yml`, and
    /// updates `.gitignore`.
    Init {
        /// Directory to initialize
        #[arg(long, default_value = ".")]
        dir: std::path::PathBuf,
    },
    /// Run a simulation
    ///
    /// Loads a spec, runs the simulation, routes events to systems,
    /// and records everything.
    Run {
        /// Write  events to stdout (instead of routing to systems)
        #[arg(long)]
        stdout: bool,
        /// Path to a spec file (instead of building from the `.rngo` directory)
        #[arg(long)]
        spec: Option<std::path::PathBuf>,
        /// Path to the `.rngo` directory
        #[arg(long, default_value = ".")]
        dir: std::path::PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { dir } => {
            if let Err(e) = init::init(&dir) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Run { stdout, dir, spec } => {
            if let Err(e) = sim::run(&dir, stdout, spec.as_deref()) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }
}
