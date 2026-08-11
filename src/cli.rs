use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "loadout",
    version,
    about = "Check a repository's local development requirements"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Commands {
    /// Inspect repository metadata and local prerequisites
    Check {
        /// Repository directory (defaults to the current directory)
        path: Option<PathBuf>,
        /// Add a one-off requirement: cmd:NAME, cmd:NAME@VERSION, or env:NAME
        #[arg(long = "require", value_name = "REQUIREMENT")]
        requirements: Vec<String>,
        /// Add a reusable built-in requirement profile (also reads LOADOUT_PROFILE)
        #[arg(long, value_enum, value_delimiter = ',', env = "LOADOUT_PROFILE")]
        profile: Vec<Profile>,
        /// Emit a machine-readable report
        #[arg(long)]
        json: bool,
        /// Disable ANSI colors in human output
        #[arg(long)]
        no_color: bool,
        /// Include only checks matching a kind (command, environment, dependency_state, connectivity) or name
        #[arg(long, value_name = "FILTER")]
        only: Vec<String>,
        /// Exclude checks matching a kind (command, environment, dependency_state, connectivity) or name
        #[arg(long, value_name = "FILTER")]
        skip: Vec<String>,
        /// Treat warnings as failures for this invocation
        #[arg(long)]
        strict: bool,
        /// Suppress passing checks in human output
        #[arg(long, conflicts_with = "json")]
        quiet: bool,
        /// Also verify configured services are reachable (makes network connections; opt-in)
        #[arg(long)]
        services: bool,
        /// Only check requirements whose source file changed relative to this git ref (e.g. origin/main)
        #[arg(long, value_name = "BASE_REF")]
        changed: Option<String>,
        /// Emit GitHub Actions ::error/::warning annotations for failing and warning checks
        #[arg(long)]
        annotate: bool,
        /// Ignore the repository's .loadoutignore file for this invocation
        #[arg(long)]
        no_ignore_file: bool,
    },
    /// Print an advisory summary of detected requirements without creating files
    Init {
        /// Repository directory (defaults to the current directory)
        path: Option<PathBuf>,
        /// Emit a machine-readable advisory
        #[arg(long)]
        json: bool,
    },
    /// Guided view that groups blockers into ordered next steps
    Doctor {
        /// Repository directory (defaults to the current directory)
        path: Option<PathBuf>,
        /// Add a one-off requirement: cmd:NAME, cmd:NAME@VERSION, or env:NAME
        #[arg(long = "require", value_name = "REQUIREMENT")]
        requirements: Vec<String>,
        /// Add a reusable built-in requirement profile (also reads LOADOUT_PROFILE)
        #[arg(long, value_enum, value_delimiter = ',', env = "LOADOUT_PROFILE")]
        profile: Vec<Profile>,
        /// Emit a machine-readable report
        #[arg(long)]
        json: bool,
        /// Disable ANSI colors in human output
        #[arg(long)]
        no_color: bool,
        /// Open each missing tool's install docs in your browser
        #[arg(long)]
        open_docs: bool,
        /// Also verify configured services are reachable (makes network connections; opt-in)
        #[arg(long)]
        services: bool,
        /// Ignore the repository's .loadoutignore file for this invocation
        #[arg(long)]
        no_ignore_file: bool,
    },
    /// Generate shell completion scripts to stdout
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Generate a roff man page to stdout
    Man,
}

#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum Shell {
    Bash,
    Elvish,
    Fish,
    Powershell,
    Zsh,
}

#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum Profile {
    Web,
    Rust,
    Python,
    Containers,
    Infra,
    Data,
}
