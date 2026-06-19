//! `mev` CLI entry point. Thin wrapper over the library: parse args, dispatch, set exit code.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "mev",
    version,
    about = "Validate and compile learn-agentic-ai.com content"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate the content tree (Phase 1: learn modules).
    Validate {
        /// Path to the content root (e.g. ../learn-ai/content/learn).
        #[arg(default_value = "../learn-ai/content/learn")]
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate { path } => match mev::validate(&path) {
            Ok(report) => {
                // Phase 0: reporter is a stub. Block E delivers the grouped human/JSON output.
                println!(
                    "validated {}: {} error(s), {} warning(s)",
                    path.display(),
                    report.error_count(),
                    report.warning_count()
                );
                if report.is_failure() {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(err) => {
                eprintln!("error: {err:#}");
                ExitCode::FAILURE
            }
        },
    }
}
