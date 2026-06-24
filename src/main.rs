//! Hawk CLI: inspect Foundry projects from the command line.

use std::path::{Path, PathBuf};

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
    /// List all abstract contracts
    Abstracts {
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// List all deployable contracts
    Contracts {
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// Show the inheritance graph of a declaration
    Inheritances {
        /// The declaration name (contract, abstract, or interface)
        decl: String,
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// List all interfaces
    Interfaces {
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// List all libraries
    Libraries {
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
}

/// Print items with a header and numbered list.
fn print_items(project: &Path, items: &[String], label: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let project_abs = std::path::absolute(project)?;
    let project_rel = project_abs.strip_prefix(&cwd).unwrap_or(&project_abs);

    println!("found {} {}\n", items.len(), label);

    for (i, line) in items.iter().enumerate() {
        println!("{}. {}", i + 1, project_rel.join(line).display());
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Inspect(args) => match args.subcommand {
            InspectSubcommand::Abstracts { project } => {
                let items = hawk::commands::abstracts::list(&project)?;
                print_items(&project, &items, "abstracts")?;
            }
            InspectSubcommand::Contracts { project } => {
                let items = hawk::commands::contracts::list(&project)?;
                print_items(&project, &items, "contracts")?;
            }
            InspectSubcommand::Inheritances { decl, project } => {
                let output = hawk::commands::inheritances::run(&decl, &project)?;
                print!("{output}");
            }
            InspectSubcommand::Interfaces { project } => {
                let items = hawk::commands::interfaces::list(&project)?;
                print_items(&project, &items, "interfaces")?;
            }
            InspectSubcommand::Libraries { project } => {
                let items = hawk::commands::libraries::list(&project)?;
                print_items(&project, &items, "libraries")?;
            }
        },
    }

    Ok(())
}
