mod project;
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
    Init {
        #[arg(long, default_value = ".")]
        dir: std::path::PathBuf,
    },
    Run {
        #[arg(long)]
        stdout: bool,
        #[arg(long, default_value = ".")]
        dir: std::path::PathBuf,
        #[arg(long)]
        spec: Option<std::path::PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { dir } => {
            if let Err(e) = project::init(&dir) {
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
