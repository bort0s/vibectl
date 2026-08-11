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
    /// Show the registry as a table
    List(ListArgs),
    /// Dump one project's manifest, ready to paste at an agent
    Show(ShowArgs),
    /// Re-read the disk and update a manifest
    Sync(SyncArgs),
    /// Take a project off your desk. Never deletes anything.
    Archive(ArchiveArgs),
    /// Put an archived project back
    Unarchive(ArchiveArgs),
    /// Generate CLAUDE.md, AGENTS.md or README.md from the manifest
    Render(RenderArgs),
    /// Manage the agents a project declares
    #[command(subcommand)]
    Agents(AgentsCommand),
}

#[derive(Debug, Args)]
pub struct RenderArgs {
    /// What to generate: claude, agents, or readme
    #[arg(value_parser = parse_render_target)]
    pub target: vibe_core::RenderTarget,

    /// Project directory
    #[arg(long, default_value = ".")]
    pub path: PathBuf,

    /// Overwrite a generated file you have since edited. The edits are lost.
    ///
    /// This will NOT overwrite a file vibe did not generate - a hand-written
    /// README stays yours whatever flag you pass (ADR-0007 §4).
    #[arg(long)]
    pub force: bool,

    #[command(flatten)]
    pub write: WriteFlags,

    #[command(flatten)]
    pub format: FormatFlags,
}

/// Reject anything outside the closed target set, and say what is valid.
///
/// The set is closed so a target name can never become a path to write to;
/// listing the alternatives is `vibectl`'s job, since core does not write prose.
fn parse_render_target(s: &str) -> Result<vibe_core::RenderTarget, String> {
    s.parse().map_err(|()| {
        format!(
            "unknown render target `{s}` - expected one of: {}",
            vibe_core::RenderTarget::ALL
                .iter()
                .map(|t| t.file_name().to_ascii_lowercase().replace(".md", ""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

/// The six agent subcommands.
///
/// `update` is the **only** one that touches the network. Every other command
/// works fully offline against whatever the store already holds, which is what
/// keeps the tool usable on a plane and keeps a network round-trip off the hot
/// path of an unrelated command (ADR-0006 §1, §7).
#[derive(Debug, Subcommand)]
pub enum AgentsCommand {
    /// Fetch the agent store. The only command that uses the network.
    Update(AgentsUpdateArgs),
    /// Show what the store offers
    List(AgentsListArgs),
    /// Show the state of this project's agents
    Status(AgentsStatusArgs),
    /// Install agents and declare them in the manifest
    Add(AgentsAddArgs),
    /// Remove agents vibe installed, and undeclare them
    Remove(AgentsRemoveArgs),
    /// Install every declared agent that is not present
    Sync(AgentsSyncArgs),
}

/// Where the store lives and what it points at.
///
/// Flattened into every agent subcommand rather than declared once as a global,
/// so a command cannot exist that reads a *different* store than the one the
/// user named.
#[derive(Debug, Args)]
pub struct StoreFlags {
    /// Upstream repository to clone the agent store from
    #[arg(long, value_name = "URL")]
    pub store_url: Option<String>,

    /// Where the agent store lives on disk
    #[arg(long, value_name = "DIR")]
    pub store_path: Option<PathBuf>,

    /// Days before the store's age is mentioned
    #[arg(long, value_name = "DAYS")]
    pub stale_after: Option<u64>,
}

#[derive(Debug, Args)]
pub struct AgentsUpdateArgs {
    #[command(flatten)]
    pub store: StoreFlags,

    #[command(flatten)]
    pub format: FormatFlags,
}

#[derive(Debug, Args)]
pub struct AgentsListArgs {
    /// Project directory, used only to mark which agents it already declares
    #[arg(default_value = ".")]
    pub path: PathBuf,

    #[command(flatten)]
    pub store: StoreFlags,

    #[command(flatten)]
    pub format: FormatFlags,
}

#[derive(Debug, Args)]
pub struct AgentsStatusArgs {
    /// Project directory
    #[arg(default_value = ".")]
    pub path: PathBuf,

    #[command(flatten)]
    pub store: StoreFlags,

    #[command(flatten)]
    pub format: FormatFlags,
}

#[derive(Debug, Args)]
pub struct AgentsAddArgs {
    /// Agent names, as `vibe agents list` shows them
    #[arg(required = true)]
    pub names: Vec<String>,

    /// Project directory
    #[arg(long, default_value = ".")]
    pub path: PathBuf,

    #[command(flatten)]
    pub force: ForceFlag,

    #[command(flatten)]
    pub store: StoreFlags,

    #[command(flatten)]
    pub write: WriteFlags,

    #[command(flatten)]
    pub format: FormatFlags,
}

#[derive(Debug, Args)]
pub struct AgentsRemoveArgs {
    /// Agent names
    #[arg(required = true)]
    pub names: Vec<String>,

    /// Project directory
    #[arg(long, default_value = ".")]
    pub path: PathBuf,

    #[command(flatten)]
    pub store: StoreFlags,

    #[command(flatten)]
    pub write: WriteFlags,

    #[command(flatten)]
    pub format: FormatFlags,
}

#[derive(Debug, Args)]
pub struct AgentsSyncArgs {
    /// Project directory
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Also rewrite installed agents whose store revision has moved
    #[arg(long)]
    pub update: bool,

    #[command(flatten)]
    pub force: ForceFlag,

    #[command(flatten)]
    pub store: StoreFlags,

    #[command(flatten)]
    pub write: WriteFlags,

    #[command(flatten)]
    pub format: FormatFlags,
}

/// The flag that overwrites an edited agent.
///
/// Its own type, flattened, so the help text stating what is at stake is
/// written once. Editing an installed agent is the *normal* reason to have
/// installed one, which makes this the single most likely way for the feature
/// to destroy user work (ADR-0006 §5).
#[derive(Debug, Args)]
pub struct ForceFlag {
    /// Overwrite agents that were edited after they were installed. The edits
    /// are lost; there is no merge.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Directory to list projects from
    #[arg(default_value = ".")]
    pub path: PathBuf,

    #[arg(long, default_value_t = 3)]
    pub depth: usize,

    /// Include archived projects, which are hidden by default
    #[arg(long)]
    pub all: bool,

    /// Only projects with this status
    #[arg(long, value_parser = parse_status)]
    pub status: Option<Status>,

    #[command(flatten)]
    pub format: FormatFlags,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Project directory
    #[arg(default_value = ".")]
    pub path: PathBuf,

    #[command(flatten)]
    pub format: FormatFlags,
}

#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Project directory
    #[arg(default_value = ".")]
    pub path: PathBuf,

    #[command(flatten)]
    pub write: WriteFlags,

    #[command(flatten)]
    pub format: FormatFlags,
}

#[derive(Debug, Args)]
pub struct ArchiveArgs {
    /// Project directory
    #[arg(default_value = ".")]
    pub path: PathBuf,

    #[command(flatten)]
    pub write: WriteFlags,

    #[command(flatten)]
    pub format: FormatFlags,
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
