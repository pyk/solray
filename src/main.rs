//! Solray CLI: inspect Foundry projects from the command line.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use solray::AbstractInspector;
use solray::ArtifactId;
use solray::CallGraphInspector;
use solray::CallPathInspector;
use solray::ContractInspector;
use solray::Erc20TransferSinkScanner;
use solray::ExternalFunctionInspector;
use solray::FunctionId;
use solray::FunctionSourceInspector;
use solray::InheritanceGraphInspector;
use solray::InterfaceInspector;
use solray::LibraryInspector;
use solray::ModifierInspector;
use solray::Project;
use solray::StorageLayoutId;
use solray::StorageLayoutInspector;

#[derive(Parser)]
#[command(name = "solray", about = "Solidity source code explorer", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Explore contract structure and details
    Inspect(InspectArgs),
    /// Search for specific code patterns across the codebase
    Scan(ScanArgs),
}

#[derive(clap::Args)]
struct ScanArgs {
    #[command(subcommand)]
    subcommand: ScanSubcommand,
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
    /// Show the call graph of a function
    CallGraph {
        /// The artifact ID (e.g. Name or File.sol:Name)
        contract: String,
        /// The function name
        function: String,
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Enable trace logging for performance diagnostics
        #[arg(short, long)]
        verbose: bool,
    },
    /// Show call paths from entry functions to a target function
    CallPath {
        /// The artifact ID (e.g. Name or File.sol:Name)
        contract: String,
        /// The target function name (or Contract::function for library functions)
        function: String,
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
    /// List all external functions from a contract ABI
    ExternalFunctions {
        /// The artifact ID (e.g. Name or File.sol:Name)
        id: String,
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Show read-only functions in the output
        #[arg(long)]
        include_read_only: bool,
    },
    /// Show the inheritance graph of a contract or interface
    InheritanceGraph {
        /// The artifact ID (e.g. Name or File.sol:Name)
        id: String,
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
    /// List all modifiers in a contract
    Modifiers {
        /// The artifact ID (e.g. Name or File.sol:Name)
        id: String,
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
    /// Show the complete resolved source code of a function
    FunctionSource {
        /// The artifact ID (e.g. Name or File.sol:Name)
        contract: String,
        /// The function name
        function: String,
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// Show the storage layout of a contract
    StorageLayout {
        /// The artifact ID (e.g. Name or File.sol:Name)
        id: String,
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
}

#[derive(Subcommand)]
enum ScanSubcommand {
    /// Scan for ERC20 transfer and safeTransfer calls.
    Erc20TransferSink {
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Scan(args) => match args.subcommand {
            ScanSubcommand::Erc20TransferSink { project } => {
                let project = Project::open(&project);
                let scanner = Erc20TransferSinkScanner::new(project);
                let output = scanner.scan()?;
                print!("{output}");
            }
        },
        Command::Inspect(args) => match args.subcommand {
            InspectSubcommand::Abstracts { project } => {
                let project = Project::open(&project);
                let inspector = AbstractInspector::new(project);
                let output = inspector.inspect()?;
                print!("{output}");
            }
            InspectSubcommand::CallGraph {
                contract,
                function,
                project,
                verbose,
            } => {
                if verbose {
                    let _ = tracing_subscriber::fmt()
                        .with_max_level(tracing::Level::TRACE)
                        .with_target(true)
                        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                        .with_writer(std::io::stderr)
                        .try_init();
                }
                let project = Project::open(&project);
                let inspector = CallGraphInspector::new(project);
                let artifact_id = ArtifactId::new(&contract);
                let function_id = FunctionId::new(artifact_id, &function);
                let output = inspector.inspect(&function_id)?;
                print!("{output}");
            }
            InspectSubcommand::CallPath {
                contract,
                function,
                project,
            } => {
                let project = Project::open(&project);
                let inspector = CallPathInspector::new(project);
                let artifact_id = ArtifactId::new(&contract);
                let function_id = FunctionId::new(artifact_id, &function);
                let output = inspector.inspect(&function_id, &function)?;
                print!("{output}");
            }
            InspectSubcommand::Contracts { project } => {
                let project = Project::open(&project);
                let inspector = ContractInspector::new(project);
                let output = inspector.inspect()?;
                print!("{output}");
            }
            InspectSubcommand::ExternalFunctions {
                id,
                project,
                include_read_only,
            } => {
                let project = Project::open(&project);
                let inspector = ExternalFunctionInspector::new(project);
                let id = ArtifactId::new(&id);
                let output = inspector.inspect(&id, include_read_only)?;
                print!("{output}");
            }
            InspectSubcommand::InheritanceGraph { id, project } => {
                let project = Project::open(&project);
                let inspector = InheritanceGraphInspector::new(project);
                let id = ArtifactId::new(&id);
                let output = inspector.inspect(&id)?;
                print!("{output}");
            }
            InspectSubcommand::Interfaces { project } => {
                let project = Project::open(&project);
                let inspector = InterfaceInspector::new(project);
                let output = inspector.inspect()?;
                print!("{output}");
            }
            InspectSubcommand::Modifiers { id, project } => {
                let project = Project::open(&project);
                let inspector = ModifierInspector::new(project);
                let id = ArtifactId::new(&id);
                let output = inspector.inspect(&id)?;
                print!("{output}");
            }
            InspectSubcommand::Libraries { project } => {
                let project = Project::open(&project);
                let inspector = LibraryInspector::new(project);
                let output = inspector.inspect()?;
                print!("{output}");
            }
            InspectSubcommand::FunctionSource {
                contract,
                function,
                project,
            } => {
                let project = Project::open(&project);
                let inspector = FunctionSourceInspector::inspect_project(project);
                let id = ArtifactId::new(&contract);
                let output = inspector.inspect(&id, &function)?;
                print!("{output}");
            }
            InspectSubcommand::StorageLayout { id, project } => {
                let project = Project::open(&project);
                let inspector = StorageLayoutInspector::new(project);
                let id = StorageLayoutId::new(&id);
                let output = inspector.inspect(&id)?;
                print!("{output}");
            }
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn version_comes_from_cargo_package_version() {
        let expected = format!("solray {}\n", env!("CARGO_PKG_VERSION"));
        assert_eq!(Cli::command().render_version(), expected);
    }
}
