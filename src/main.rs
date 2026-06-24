//! Hawk CLI: inspect Foundry projects from the command line.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "hawk", about = "Inspect Foundry projects")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect a Foundry project.
    Inspect(InspectArgs),
}

#[derive(clap::Args)]
struct InspectArgs {
    #[command(subcommand)]
    subcommand: InspectSubcommand,
}

#[derive(Subcommand)]
enum InspectSubcommand {
    /// List all deployable contracts
    Contracts {
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Inspect(args) => match args.subcommand {
            InspectSubcommand::Contracts { project } => {
                let contracts = hawk::commands::contracts::list(&project).unwrap_or_else(|e| {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                });

                for decl in &contracts {
                    println!("{}", decl.name);
                }
            }
        },
    }
}
