//! Solray CLI: inspect Foundry projects from the command line.

use std::fmt::Display;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use solray::AbstractInspector;
use solray::ArtifactId;
use solray::AssetTransferScanner;
use solray::CallGraphInspector;
use solray::CallPathInspector;
use solray::ContractInspector;
use solray::Erc20TransferSinkScanner;
use solray::ExternalFunctionInspector;
use solray::FunctionId;
use solray::FunctionSourceInspector;
use solray::InheritanceGraphInspector;
use solray::InterfaceGenerator;
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
    /// Generate Solidity source from a contract's artifact
    Gen(GenArgs),
    /// Search for specific code patterns across the codebase
    Scan(ScanArgs),
}

#[derive(clap::Args)]
struct ScanArgs {
    #[command(subcommand)]
    subcommand: ScanSubcommand,
}

#[derive(clap::Args)]
struct GenArgs {
    #[command(subcommand)]
    subcommand: GenSubcommand,
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
        /// Enable debug logging for resolution tracing
        #[arg(short, long)]
        debug: bool,
    },
    /// Show call paths from entry functions to a target function
    CallPath {
        /// The artifact ID (e.g. Name or File.sol:Name)
        contract: String,
        /// The target function name
        function: String,
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Enable debug logging for resolution tracing
        #[arg(short, long)]
        debug: bool,
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
        /// Enable debug logging for resolution tracing
        #[arg(short, long)]
        debug: bool,
    },
    /// Show the inheritance graph of a contract or interface
    InheritanceGraph {
        /// The artifact ID (e.g. Name or File.sol:Name)
        id: String,
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Show debug logs while resolving the inheritance graph
        #[arg(long)]
        debug: bool,
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
        /// Show debug logs while resolving modifiers
        #[arg(long)]
        debug: bool,
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
        /// Show debug logs while resolving function source
        #[arg(long)]
        debug: bool,
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
        /// Show debug logs while scanning
        #[arg(long)]
        debug: bool,
    },
    /// Scan for asset transfer calls (ERC20 transfers, ETH transfers, and
    /// low-level calls with value).
    AssetTransfers {
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Show debug logs while scanning
        #[arg(long)]
        debug: bool,
    },
}

#[derive(Subcommand)]
enum GenSubcommand {
    /// Generate a Solidity interface for a contract
    Interface {
        /// The artifact ID (e.g. Name or File.sol:Name)
        contract: String,
        /// Path to the Foundry project
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Gen(args) => match args.subcommand {
            GenSubcommand::Interface { contract, project } => {
                let project = Project::open(&project);
                let generator = InterfaceGenerator::new(project);
                let id = ArtifactId::new(&contract);
                let output = generator.generate(&id)?;
                print_output(&output);
            }
        },
        Command::Scan(args) => match args.subcommand {
            ScanSubcommand::Erc20TransferSink { project, debug } => {
                if debug {
                    let _ = tracing_subscriber::fmt()
                        .with_max_level(tracing::Level::DEBUG)
                        .with_target(true)
                        .with_writer(std::io::stderr)
                        .try_init();
                }
                let project = Project::open(&project);
                let scanner = Erc20TransferSinkScanner::new(project);
                let output = scanner.scan()?;
                print_output(&output);
            }
            ScanSubcommand::AssetTransfers { project, debug } => {
                if debug {
                    let _ = tracing_subscriber::fmt()
                        .with_max_level(tracing::Level::DEBUG)
                        .with_target(true)
                        .with_writer(std::io::stderr)
                        .try_init();
                }
                let project = Project::open(&project);
                let scanner = AssetTransferScanner::new(project);
                let output = scanner.scan()?;
                print_output(&output);
            }
        },
        Command::Inspect(args) => match args.subcommand {
            InspectSubcommand::Abstracts { project } => {
                let project = Project::open(&project);
                let inspector = AbstractInspector::new(project);
                let output = inspector.inspect()?;
                print_output(&output);
            }
            InspectSubcommand::CallGraph {
                contract,
                function,
                project,
                verbose,
                debug,
            } => {
                if verbose {
                    let _ = tracing_subscriber::fmt()
                        .with_max_level(tracing::Level::TRACE)
                        .with_target(true)
                        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                        .with_writer(std::io::stderr)
                        .try_init();
                } else if debug {
                    let _ = tracing_subscriber::fmt()
                        .with_max_level(tracing::Level::DEBUG)
                        .with_target(true)
                        .with_writer(std::io::stderr)
                        .try_init();
                }
                let project = Project::open(&project);
                let inspector = CallGraphInspector::new(project);
                let artifact_id = ArtifactId::new(&contract);
                let function_id = FunctionId::new(artifact_id, &function);
                let output = inspector.inspect(&function_id)?;
                print_output(&output);
            }
            InspectSubcommand::CallPath {
                contract,
                function,
                project,
                debug,
            } => {
                if debug {
                    let _ = tracing_subscriber::fmt()
                        .with_max_level(tracing::Level::DEBUG)
                        .with_target(true)
                        .with_writer(std::io::stderr)
                        .try_init();
                }
                let project = Project::open(&project);
                let inspector = CallPathInspector::new(project);
                let artifact_id = ArtifactId::new(&contract);
                let function_id = FunctionId::new(artifact_id, &function);
                let output = inspector.inspect(&function_id, &function)?;
                print_output(&output);
            }
            InspectSubcommand::Contracts { project } => {
                let project = Project::open(&project);
                let inspector = ContractInspector::new(project);
                let output = inspector.inspect()?;
                print_output(&output);
            }
            InspectSubcommand::ExternalFunctions { id, project, debug } => {
                if debug {
                    let _ = tracing_subscriber::fmt()
                        .with_max_level(tracing::Level::DEBUG)
                        .with_target(true)
                        .with_writer(std::io::stderr)
                        .try_init();
                }
                let project = Project::open(&project);
                let inspector = ExternalFunctionInspector::new(project);
                let id = ArtifactId::new(&id);
                let output = inspector.inspect(&id)?;
                print_output(&output);
            }
            InspectSubcommand::InheritanceGraph { id, project, debug } => {
                if debug {
                    let _ = tracing_subscriber::fmt()
                        .with_max_level(tracing::Level::DEBUG)
                        .with_target(true)
                        .with_writer(std::io::stderr)
                        .try_init();
                }
                let project = Project::open(&project);
                let inspector = InheritanceGraphInspector::new(project);
                let id = ArtifactId::new(&id);
                let output = inspector.inspect(&id)?;
                print_output(&output);
            }
            InspectSubcommand::Interfaces { project } => {
                let project = Project::open(&project);
                let inspector = InterfaceInspector::new(project);
                let output = inspector.inspect()?;
                print_output(&output);
            }
            InspectSubcommand::Modifiers { id, project, debug } => {
                if debug {
                    let _ = tracing_subscriber::fmt()
                        .with_max_level(tracing::Level::DEBUG)
                        .with_target(true)
                        .with_writer(std::io::stderr)
                        .try_init();
                }
                let project = Project::open(&project);
                let inspector = ModifierInspector::new(project);
                let id = ArtifactId::new(&id);
                let output = inspector.inspect(&id)?;
                print_output(&output);
            }
            InspectSubcommand::Libraries { project } => {
                let project = Project::open(&project);
                let inspector = LibraryInspector::new(project);
                let output = inspector.inspect()?;
                print_output(&output);
            }
            InspectSubcommand::FunctionSource {
                contract,
                function,
                project,
                debug,
            } => {
                if debug {
                    let _ = tracing_subscriber::fmt()
                        .with_max_level(tracing::Level::DEBUG)
                        .with_target(true)
                        .with_writer(std::io::stderr)
                        .try_init();
                }
                let project = Project::open(&project);
                let inspector = FunctionSourceInspector::inspect_project(project);
                let id = ArtifactId::new(&contract);
                let output = inspector.inspect(&id, &function)?;
                print_output(&output);
            }
            InspectSubcommand::StorageLayout { id, project } => {
                let project = Project::open(&project);
                let inspector = StorageLayoutInspector::new(project);
                let id = StorageLayoutId::new(&id);
                let output = inspector.inspect(&id)?;
                print_output(&output);
            }
        },
    }

    Ok(())
}

/// Print command output to stdout, exiting quietly when the consumer closes
/// the pipe early (e.g. `solray inspect ... | head`). The Rust runtime
/// ignores `SIGPIPE`, so a closed stdout would otherwise panic with a
/// broken-pipe error.
fn print_output(output: impl Display) {
    let rendered = output.to_string();
    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(rendered.as_bytes()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("error: failed to write output: {error}");
            std::process::exit(1);
        }
    }
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
