//! `mev` CLI entry point. Thin wrapper over the library: parse args, dispatch, set exit code.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use mev::Severity;
use mev::theme;

#[derive(Parser)]
#[command(
    name = "mev",
    version,
    about = "Validate Markdown/MDX content: learn-agentic-ai.com content and Bastion Brain OKF frontmatter"
)]
struct Cli {
    /// Emit machine-readable JSON envelope to stdout instead of a human summary.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate the learn-ai content tree (Phase 1: learn modules).
    Validate {
        /// Path to the content root (e.g. ../learn-ai/content/learn).
        #[arg(default_value = "../learn-ai/content/learn")]
        path: PathBuf,
    },
    /// Validate the Bastion Brain repo for OKF frontmatter compliance (Phase 2).
    /// With --sync, also checks cross-repo synced_from watermark integrity (Phase 3, Block M).
    /// With --graph, also checks the global scope:doc_id knowledge-graph integrity (Phase 3, Block J).
    /// With --state, also checks each repo's planning/state.json schema and cross-repo block graph (Phase 3, Block P).
    /// With --links, also checks markdown, file://, and [[wikilink]] references for dead or moved targets (Phase 3, Block K).
    ValidateBrain {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Also run the cross-repo sync watermark check: compares each sub-repo's
        /// planning/status.md `timestamp` against its brain cache doc's `synced_from`.
        /// A mismatch emits an E_SYNC_DRIFT error (exit 1).
        #[arg(long)]
        sync: bool,
        /// Also run the global scope:doc_id knowledge-graph integrity check: flags duplicate
        /// canonical ids (E_GRAPH_DUPLICATE_DOC_ID), dangling related: edges
        /// (E_GRAPH_DANGLING_RELATED), and related: entries pointing at leaf files
        /// (W_GRAPH_LEAF_TARGET). Graph errors cause exit 1; the leaf warning alone does not.
        #[arg(long)]
        graph: bool,
        /// Also run the state.json schema and block-dependency graph integrity check: validates
        /// each repo's planning/state.json against the canonical schema, checks for dangling
        /// blocked_by references (E_STATE_DANGLING_BLOCKED_BY), unknown repos
        /// (E_STATE_UNKNOWN_REPO), and flags rollup drift (W_STATE_ROLLUP_DRIFT).
        /// State errors cause exit 1; drift-only warnings exit 0.
        #[arg(long)]
        state: bool,
        /// Also run the link-integrity pass: flags dead markdown [text](path) links
        /// (E_LINK_DEAD_MARKDOWN), dead file:// URIs (E_LINK_DEAD_FILE_URI), dangling
        /// [[wikilink]] slugs (E_LINK_DANGLING_WIKILINK), and references still pointing
        /// at paths listed in .brain-moves-pending (E_LINK_MOVED_REFERENCE).
        /// Link errors cause exit 1.
        #[arg(long)]
        links: bool,
    },
}

fn print_diagnostic(d: &mev::Diagnostic) {
    let sev = match d.severity {
        Severity::Error => theme::severity_error("error"),
        Severity::Warning => theme::severity_warning("warning"),
    };
    println!(
        "{} [{}] {} — {}",
        sev,
        theme::locator(&d.locator),
        d.file.display(),
        theme::message(&d.message),
    );
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate { path } => match mev::validate(&path) {
            Ok(report) => {
                if cli.json {
                    let envelope = mev::JsonReport::new("learn-ai", &path, &report);
                    match envelope.to_json() {
                        Ok(s) => println!("{s}"),
                        Err(err) => {
                            eprintln!("error serializing JSON: {err:#}");
                            return ExitCode::FAILURE;
                        }
                    }
                } else {
                    for d in &report.diagnostics {
                        print_diagnostic(d);
                    }
                    println!(
                        "validated {}: {} error(s), {} warning(s)",
                        path.display(),
                        report.error_count(),
                        report.warning_count()
                    );
                }
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
        Command::ValidateBrain {
            path,
            sync,
            graph,
            state,
            links,
        } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let result = if state {
                mev::validate_brain_state(&root)
            } else if graph {
                mev::validate_brain_graph(&root)
            } else if sync {
                mev::validate_brain_sync(&root)
            } else if links {
                mev::validate_brain_links(&root)
            } else {
                mev::validate_brain(&root)
            };
            match result {
                Ok(report) => {
                    if cli.json {
                        let envelope = mev::JsonReport::new("brain", &root, &report);
                        match envelope.to_json() {
                            Ok(s) => println!("{s}"),
                            Err(err) => {
                                eprintln!("error serializing JSON: {err:#}");
                                return ExitCode::FAILURE;
                            }
                        }
                    } else {
                        for d in &report.diagnostics {
                            print_diagnostic(d);
                        }
                        println!(
                            "validated {}: {} error(s), {} warning(s)",
                            root.display(),
                            report.error_count(),
                            report.warning_count()
                        );
                    }
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
            }
        }
    }
}
