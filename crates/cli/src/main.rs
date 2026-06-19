use clap::{Parser, Subcommand};
use rngo_sim::Dialect;
use std::error::Error;
use std::fmt;
use std::fs;

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
    Run,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Sim { command } => match command {
            SimCommands::Run => {
                if let Err(e) = run() {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        },
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let content = fs::read_to_string(".rngo/spec.yml")?;
    let value: serde_json::Value = serde_yaml::from_str(&content)?;

    let simulation_builder = Dialect::core()
        .parse_simulation_json(value)
        .map_err(join_errors)?;

    let simulation = simulation_builder.build().map_err(join_errors)?;

    for event in simulation {
        println!("{}", serde_json::to_string(&event)?);
    }

    Ok(())
}

fn join_errors<E: fmt::Display>(errors: Vec<E>) -> String {
    errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n")
}
