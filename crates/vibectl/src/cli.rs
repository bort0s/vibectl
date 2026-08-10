use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use vibe_core::Status;

#[derive(Debug, Parser)]
#[command(
    name = "vibe",
    version,
    about = "A registry for the half-finished projects you already have",
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Scaffold a new project and register it
    New(NewArgs),
    /// Index projects that already exist on disk
    Scan(ScanArgs),
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Directory to index. Its subdirectories are searched for projects.
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// How deep below the root to look for project directories
    #[arg(long, default_value_t = 3)]
    pub depth: usize,

    /// Show values that were found but not written: weak evidence and
    /// unresolved conflicts
    #[arg(long)]
    pub suggestions: bool,

    #[command(flatten)]
    pub format: FormatFlags,
}

/// Flags every write command carries.
///
/// Defined once and flattened rather than repeated per subcommand, so
/// `--dry-run` cannot be missing from a command that writes.
#[derive(Debug, Args)]
pub struct WriteFlags {
    /// Show what would change without touching anything
    #[arg(long, global = false)]
    pub dry_run: bool,
}

/// Flags every machine-readable command carries.
#[derive(Debug, Args)]
pub struct FormatFlags {
    /// Emit JSON instead of a human-readable summary
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct NewArgs {
    /// Project name. Becomes the directory name.
    pub name: String,

    /// Where to create the project directory
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub path: PathBuf,

    /// One-line description
    #[arg(long)]
    pub description: Option<String>,

    /// Lifecycle status
    #[arg(long, default_value = "active", value_parser = parse_status)]
    pub status: Status,

    #[command(flatten)]
    pub write: WriteFlags,

    #[command(flatten)]
    pub format: FormatFlags,
}

/// Accepts unknown values rather than rejecting them.
///
/// `Status` has an `Other` variant precisely so a value written by a future
/// build round-trips; rejecting one at the CLI would make this build stricter
/// than the file format it reads, which is the wrong way round.
fn parse_status(s: &str) -> Result<Status, std::convert::Infallible> {
    s.parse()
}
