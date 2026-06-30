mod sim;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rngo")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Sim {
        #[command(subcommand)]
        command: SimCommands,
    },
}

#[derive(Subcommand)]
enum SimCommands {
    Run {
        #[arg(long)]
        stdout: bool,
        #[arg(long, default_value = ".")]
        dir: std::path::PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Sim { command } => match command {
            SimCommands::Run { stdout, dir } => {
                if let Err(e) = sim::run(&dir, stdout) {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        },
    }
}
