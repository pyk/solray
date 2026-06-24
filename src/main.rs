//! Hawk CLI: inspect Foundry projects from the command line.

use std::path::PathBuf;

use anyhow::Result;
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Inspect(args) => match args.subcommand {
            InspectSubcommand::Contracts { project } => {
                let contracts = hawk::commands::contracts::list(&project)?;
                let cwd = std::env::current_dir()?;
                let project_abs = std::path::absolute(&project)?;
                let project_rel = project_abs.strip_prefix(&cwd).unwrap_or(&project_abs);

                for line in &contracts {
                    println!("{}", project_rel.join(line).display());
                }
            }
        },
    }

    Ok(())
}
