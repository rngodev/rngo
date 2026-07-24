mod init;
mod sim;
mod skills;

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
    /// Manage the rngo coding agent integration
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },
}

#[derive(Subcommand)]
enum AgentCommands {
    /// Manage rngo agent skills
    Skills {
        #[command(subcommand)]
        command: SkillsCommands,
    },
}

#[derive(Subcommand)]
enum SkillsCommands {
    /// Download the latest rngo agent skills and install them
    ///
    /// Idempotent: any previously installed `rngo-` skills in the target
    /// directory are replaced with the latest release.
    Install {
        /// Install into the user's home directory instead of the project
        #[arg(long)]
        global: bool,
        /// Coding agent to install skills for
        #[arg(long)]
        agent: Option<skills::AgentDir>,
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
        Commands::Agent { command } => match command {
            AgentCommands::Skills { command } => match command {
                SkillsCommands::Install { global, agent } => {
                    if let Err(e) = skills::install(std::path::Path::new("."), global, agent) {
                        eprintln!("error: {e}");
                        std::process::exit(1);
                    }
                }
            },
        },
    }
}
