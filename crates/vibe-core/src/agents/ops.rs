//! The six `vibe agents` commands, as plans.
//!
//! Every one of them is a function of [`crate::agents::AgentState`], which is
//! computed in exactly one place. Nothing here re-derives ownership — it asks
//! [`AgentState::is_ours_to_touch`] and [`AgentState::needs_force_to_overwrite`]
//! and does what they say. That is what keeps ADR-0006 §5's table a table rather
//! than ten scattered conditionals that drift apart.
//!
//! # Write ordering
//!
//! ADR-0006 §4: **the agent file lands first, the lockfile second**, because a
//! crash between them must not manufacture a false claim of ownership. `remove`
//! inverts it. [`crate::plan::apply`] executes ops in order, so ordering the
//! `Vec<FileOp>` correctly *is* the mechanism — which means it is worth
//! restating that the window is contained, not closed. Every state it can
//! produce is named in §5 and recovers by re-running the same command.
//!
//! The manifest edit goes last in both directions. A crash before it leaves an
//! installed-but-`Undeclared` agent, which §5 says to leave alone and suggest
//! `add` for; that is the mildest of the reachable half-states.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::lock::{self, LockedAgent, Lockfile};
use super::state::{AgentReport, AgentState, InstalledAgent, StoreAgent, compute};
use super::store::{Staleness, Store, StoreConfig, UpdateReport};
use crate::config::manifest_path;
use crate::error::CoreError;
use crate::manifest::{EditReason, FieldEdit, ManifestDocument};
use crate::plan::{FileOp, PlanIntent, WritePlan};
use crate::registry::Registry;

/// Where an installed agent goes. Project-relative.
///
/// Recorded into the lockfile per install rather than only derived here, so a
/// future change to this layout does not orphan every existing install
/// (ADR-0006 §3).
#[must_use]
pub fn install_path(name: &str) -> String {
    format!(".claude/agents/{name}.md")
}

/// Something a command declined to do, and why.
///
/// A refusal is **not** an error: `sync` installs what it can and reports what
/// it cannot, and a single missing agent must never fail the whole command
/// (ADR-0006 §6). The command's exit code becomes `Partial`, not `Failure`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Refusal {
    pub name: String,
    pub state: AgentState,
    /// A stable reason string. Prose lives in `vibectl`.
    pub why: &'static str,
}

/// A planned agent operation, with everything the caller needs to report it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AgentPlan {
    pub plan: WritePlan,
    /// The state of every agent this project knows about, before the plan runs.
    pub report: AgentReport,
    pub refused: Vec<Refusal>,
    /// Carried on the plan so the store-age line can be printed by whichever
    /// command produced it — ADR-0006 §6 requires it *whenever* anything is
    /// `NotInStore`, and the reason it is required is that "this agent does not
    /// exist" and "this machine has not fetched for twelve days" are different
    /// claims.
    pub staleness: Staleness,
}

impl AgentPlan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plan.is_empty()
    }

    /// Whether the run should report a partial result.
    #[must_use]
    pub fn partial(&self) -> bool {
        !self.refused.is_empty()
    }

    /// Whether the `NotInStore` case of ADR-0006 §6 applies.
    ///
    /// Reporting a name the store lacks without saying the store is twelve days
    /// old turns a fact about this machine into a claim about the project. §6
    /// therefore requires the age line here **regardless of the usual quiet
    /// rules**, which is stronger than §7's general "a command that read the
    /// store says when it is stale".
    ///
    /// Today those two conditions produce the same output, because there is no
    /// quiet flag for §6 to override and §7 already covers every stale store.
    /// This predicate is kept, and kept separate, because the moment a `--quiet`
    /// lands the two stop agreeing: §7's line may be suppressed and this one
    /// must not be.
    #[must_use]
    pub fn store_age_must_not_be_suppressed(&self) -> bool {
        self.report.any(AgentState::NotInStore) && self.staleness.worth_reporting()
    }
}

/// What `agents status` answers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AgentStatus {
    pub report: AgentReport,
    pub staleness: Staleness,
    pub store_rev: Option<String>,
}

/// One agent the store offers, for `agents list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StoreListing {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source_path: String,
    /// True when this project already declares it.
    pub declared: bool,
}

/// What `agents list` answers: what the store holds, and how old that is.
///
/// The age travels with the listing rather than beside it, because ADR-0006 §7
/// applies to *any* command that read the store and `list` is the one where it
/// matters most. A bare `Vec<StoreListing>` made that unimplementable: a user
/// reading a list of agents had no way to tell it was twelve days old, which is
/// the same error as reporting "this agent does not exist" when the truth is
/// "this machine has not fetched since Tuesday".
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AgentCatalogue {
    pub listings: Vec<StoreListing>,
    pub staleness: Staleness,
}

impl Registry {
    /// Clone or fast-forward the store. **The only command that uses the
    /// network** (ADR-0006 §1).
    pub fn agents_update_store(&self, store: &StoreConfig) -> Result<UpdateReport, CoreError> {
        Store::new(store, self.runner()).update()
    }

    /// Everything the store offers, offline, with the store's age.
    pub fn agents_list(
        &self,
        project_dir: Option<&Path>,
        store: &StoreConfig,
        today_utc: &str,
    ) -> Result<AgentCatalogue, CoreError> {
        let handle = Store::new(store, self.runner());
        let declared = match project_dir {
            Some(dir) => declared_agents(dir).unwrap_or_default(),
            None => BTreeSet::new(),
        };

        let mut out: Vec<StoreListing> = handle
            .load()?
            .into_values()
            .map(|a| {
                // Read the description from the file rather than carrying it
                // through `StoreAgent`, which exists for state computation and
                // has no business growing display fields.
                let description = handle
                    .read_agent(&a)
                    .ok()
                    .and_then(|bytes| super::store::parse_frontmatter(&bytes))
                    .and_then(|f| f.description);
                StoreListing {
                    declared: declared.contains(&a.name),
                    name: a.name,
                    description,
                    source_path: a.source_path,
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(AgentCatalogue {
            listings: out,
            staleness: handle.staleness(today_utc),
        })
    }

    /// The state table for one project.
    pub fn agents_status(
        &self,
        project_dir: &Path,
        store: &StoreConfig,
        today_utc: &str,
    ) -> Result<AgentStatus, CoreError> {
        let project_dir = &crate::registry::absolutize(project_dir)?;
        let handle = Store::new(store, self.runner());
        let store_agents = handle.load()?;
        let declared = declared_agents(project_dir)?;
        let lock_load = lock::load(project_dir);

        Ok(AgentStatus {
            report: compute(&lock_load, &store_agents, &declared, project_dir),
            staleness: handle.staleness(today_utc),
            store_rev: handle.head_rev().ok(),
        })
    }

    /// Install named agents and declare them.
    pub fn plan_agents_add(
        &self,
        project_dir: &Path,
        names: &[String],
        force: bool,
        store: &StoreConfig,
        today_utc: &str,
    ) -> Result<AgentPlan, CoreError> {
        let ctx = Context::open(self, project_dir, store, today_utc)?;
        let mut wanted: BTreeSet<String> = ctx.declared.clone();
        wanted.extend(names.iter().cloned());

        let mut b = Builder::new(&ctx);
        for name in names {
            b.install(name, force, InstallReason::Explicit);
        }
        b.declare(&wanted);
        b.finish(PlanIntent::AgentsAdd)
    }

    /// Install everything the manifest declares and this project does not have.
    ///
    /// `overwrite` is what separates `sync` from `update`: `sync` fills gaps,
    /// `update` also rewrites agents whose store revision has moved. Neither
    /// touches a `Modified` agent without `force`.
    pub fn plan_agents_sync(
        &self,
        project_dir: &Path,
        overwrite: bool,
        force: bool,
        store: &StoreConfig,
        today_utc: &str,
    ) -> Result<AgentPlan, CoreError> {
        let ctx = Context::open(self, project_dir, store, today_utc)?;
        let mut b = Builder::new(&ctx);

        for agent in &ctx.report.agents {
            match agent.state {
                // What `sync` is for.
                AgentState::Declared | AgentState::Missing => {
                    b.install(&agent.name, force, InstallReason::Declared);
                }
                AgentState::Installed if overwrite && agent.outdated => {
                    b.install(&agent.name, force, InstallReason::Outdated);
                }
                AgentState::Installed => {}
                AgentState::Modified => {
                    if force {
                        b.install(&agent.name, force, InstallReason::Declared);
                    } else {
                        b.refuse(
                            &agent.name,
                            agent.state,
                            "edited since it was installed; overwriting would lose the edit",
                        );
                    }
                }
                // Routed through `install` rather than refused here, so it gets
                // the same exact-content adoption `add` does. That is what lets
                // `sync` also recover the §4 crash state, and it is why this
                // arm is not the "ignores entirely" §5's table describes: a
                // declared agent whose file we will not touch is worth saying
                // out loud, not passing over in silence.
                AgentState::PresentUnowned => {
                    b.install(&agent.name, force, InstallReason::Declared);
                }
                AgentState::NotInStore => b.refuse(
                    &agent.name,
                    agent.state,
                    "the store does not have an agent with this name",
                ),
                AgentState::Unverifiable => b.refuse(
                    &agent.name,
                    agent.state,
                    "ownership cannot be established from the lockfile",
                ),
                // Left alone on purpose. Upstream deleting something is not the
                // user's instruction to delete their copy, and an undeclared
                // agent usually means an uncommitted manifest.
                //
                // No wildcard arm, deliberately. `AgentState` is
                // `#[non_exhaustive]` for downstream crates but exhaustive
                // here, so an eleventh state breaks this match and forces
                // someone to decide what `sync` does about it. A new state
                // silently defaulting to "do nothing" is how a ten-row table
                // becomes a nine-row table nobody noticed.
                AgentState::Orphaned | AgentState::RenamedUpstream | AgentState::Undeclared => {}
            }
        }
        // `sync` never edits the manifest. Declaring is the user's act
        // (ADR-0006 §6).
        b.finish(PlanIntent::AgentsSync)
    }

    /// Remove named agents, if they are ours.
    pub fn plan_agents_remove(
        &self,
        project_dir: &Path,
        names: &[String],
        store: &StoreConfig,
        today_utc: &str,
    ) -> Result<AgentPlan, CoreError> {
        let ctx = Context::open(self, project_dir, store, today_utc)?;
        let mut b = Builder::new(&ctx);

        // Undeclare only what was actually removed.
        //
        // Found by a test, and worth stating because the obvious version is
        // wrong in a way that is easy to miss: undeclaring a name whose file we
        // *refused* to delete leaves an unowned file behind with nothing
        // pointing at it, which demotes it from `PresentUnowned` (reported, so
        // the user can decide) to `Foreign` (not listed at all). A refusal must
        // leave the world as it found it, not half-apply the request.
        let mut remaining = ctx.declared.clone();
        for name in names {
            if b.remove(name) {
                remaining.remove(name);
            }
        }
        if remaining != ctx.declared {
            b.declare(&remaining);
        }
        b.finish(PlanIntent::AgentsRemove)
    }
}

/// Everything a plan builder reads, gathered once.
struct Context<'a> {
    project_dir: PathBuf,
    store_agents: BTreeMap<String, StoreAgent>,
    declared: BTreeSet<String>,
    report: AgentReport,
    locked: BTreeMap<String, LockedAgent>,
    lock_version: crate::SchemaVersion,
    staleness: Staleness,
    handle: Store<'a>,
    installed_at: String,
}

impl<'a> Context<'a> {
    fn open(
        registry: &'a Registry,
        project_dir: &Path,
        store: &'a StoreConfig,
        today_utc: &str,
    ) -> Result<Self, CoreError> {
        let project_dir = crate::registry::absolutize(project_dir)?;
        let handle = Store::new(store, registry.runner());
        let store_agents = handle.load()?;
        let declared = declared_agents(&project_dir)?;
        let lock_load = lock::load(&project_dir);

        // The one hard stop. A lockfile we cannot read means we cannot tell
        // what we own, and §5's rule is that we never touch what we do not own
        // — so nothing that writes may proceed. Refusing the write is what
        // keeps the ownership rule true rather than merely stated.
        if !lock_load.writable() {
            return Err(CoreError::OwnershipUnknown {
                why: lock_load
                    .note()
                    .unwrap_or_else(|| "the agent lockfile could not be read".to_owned()),
            });
        }

        let report = compute(&lock_load, &store_agents, &declared, &project_dir);
        let (locked, lock_version) = lock_load.usable().map_or_else(
            || (BTreeMap::new(), lock::LOCK_VERSION),
            |l| (l.agents.clone(), l.version),
        );

        Ok(Self {
            staleness: handle.staleness(today_utc),
            project_dir,
            store_agents,
            declared,
            report,
            locked,
            lock_version,
            handle,
            // RFC 3339 UTC, at day resolution. Diagnostic only — never compared
            // for staleness (ADR-0006 §3), so a clock injected for tests is
            // enough and there is no reason to reach for a wall clock here.
            installed_at: format!("{today_utc}T00:00:00Z"),
        })
    }

    fn state_of(&self, name: &str) -> Option<AgentState> {
        self.report
            .agents
            .iter()
            .find(|a| a.name == name)
            .map(|a: &InstalledAgent| a.state)
    }
}

#[derive(Debug, Clone, Copy)]
enum InstallReason {
    Explicit,
    Declared,
    Outdated,
}

/// Accumulates ops in the order ADR-0006 §4 requires.
struct Builder<'a> {
    ctx: &'a Context<'a>,
    /// Agent-file writes and deletions, in the order they must happen relative
    /// to the lockfile.
    file_ops: Vec<FileOp>,
    /// True when a removal is in the batch, which inverts the ordering.
    removing: bool,
    lock: Lockfile,
    lock_touched: bool,
    manifest_installed: Option<Vec<String>>,
    refused: Vec<Refusal>,
}

impl<'a> Builder<'a> {
    fn new(ctx: &'a Context<'a>) -> Self {
        Self {
            ctx,
            file_ops: Vec::new(),
            removing: false,
            lock: Lockfile {
                version: ctx.lock_version,
                agents: ctx.locked.clone(),
            },
            lock_touched: false,
            manifest_installed: None,
            refused: Vec::new(),
        }
    }

    fn refuse(&mut self, name: &str, state: AgentState, why: &'static str) {
        self.refused.push(Refusal {
            name: name.to_owned(),
            state,
            why,
        });
    }

    /// Whether an unowned file is byte-identical to the store's copy.
    ///
    /// The only condition under which ownership is taken of a file we have no
    /// lock entry for. Compares content, never names or timestamps: the claim
    /// being made is "writing this file would change nothing", and only the
    /// bytes can support it.
    fn adoptable_on_exact_match(&self, name: &str) -> bool {
        let Some(agent) = self.ctx.store_agents.get(name) else {
            return false;
        };
        let path = self.ctx.project_dir.join(install_path(name));
        std::fs::read(&path)
            .map(|on_disk| lock::content_hash(&on_disk) == agent.content_hash)
            .unwrap_or(false)
    }

    fn install(&mut self, name: &str, force: bool, reason: InstallReason) {
        let state = self.ctx.state_of(name);

        // Ownership first, always. A file we did not write is never adopted,
        // whatever the user asked for — silently taking ownership is how the
        // next `update` overwrites somebody's work.
        //
        // **With one exception, and it is not a guess.** If the file's content
        // is byte-identical to what the store currently holds for that name,
        // adopting it changes nothing on disk: writing the file again would be
        // a no-op, so the only thing the lock entry adds is a record of a state
        // that already exists.
        //
        // The exception exists because ADR-0006 §4's *write ordering* depends
        // on it. The ordering argument is "file first, lockfile second, because
        // file-without-entry merely loses a record and `add` is idempotent".
        // Refusing every `PresentUnowned` would make file-without-entry
        // unrecoverable — the user left holding a file vibe will neither adopt
        // nor overwrite — and the ordering would lose its justification.
        //
        // Residual, stated rather than discovered: a hand-written file that
        // happens to be byte-identical to the store's version becomes ours, and
        // `remove` would then delete it. Acceptable, because byte-identical to
        // the store means the content is recoverable from the store by
        // definition.
        if state == Some(AgentState::PresentUnowned) && !self.adoptable_on_exact_match(name) {
            self.refuse(
                name,
                AgentState::PresentUnowned,
                "a file vibe did not install, and its content differs from the store's; \
                 it will not be adopted",
            );
            return;
        }
        if state.is_some_and(AgentState::needs_force_to_overwrite) && !force {
            self.refuse(
                name,
                state.unwrap_or(AgentState::Modified),
                "edited since it was installed; re-run with --force to overwrite",
            );
            return;
        }

        let Some(agent) = self.ctx.store_agents.get(name) else {
            self.refuse(
                name,
                AgentState::NotInStore,
                "the store does not have an agent with this name",
            );
            return;
        };

        let Ok(bytes) = self.ctx.handle.read_agent(agent) else {
            self.refuse(
                name,
                AgentState::NotInStore,
                "the store lists this agent but its file could not be read",
            );
            return;
        };
        let Ok(contents) = String::from_utf8(bytes.clone()) else {
            self.refuse(
                name,
                AgentState::NotInStore,
                "the store's copy is not valid UTF-8; vibe copies agents verbatim \
                 and will not transcode one",
            );
            return;
        };

        let rel = install_path(name);
        let path = self.ctx.project_dir.join(&rel);
        let current = std::fs::read(&path).ok();

        // Already exactly right. `add` on an installed agent is a no-op, which
        // is what makes the §4 crash-recovery path ("run `add` again") safe.
        let already = current.as_deref() == Some(bytes.as_slice());
        if !already {
            self.file_ops.push(match &current {
                // The file is on disk and differs. An update carries both sides
                // so `apply` can verify nothing moved underneath the plan.
                Some(before) => FileOp::UpdateFile {
                    path,
                    before: String::from_utf8_lossy(before).into_owned(),
                    after: contents,
                    reason: match reason {
                        InstallReason::Outdated => EditReason::FieldsUpdated,
                        _ => EditReason::Created,
                    },
                },
                None => FileOp::CreateFile { path, contents },
            });
        }

        // The lock entry is rewritten even when the bytes matched, because a
        // matching file with a stale `source_rev` is exactly the half-finished
        // `update` of §4a and re-running must finish it.
        let entry = LockedAgent {
            name: name.to_owned(),
            path: rel,
            content_hash: lock::content_hash(&bytes),
            source_rev: agent.rev.clone(),
            source_path: agent.source_path.clone(),
            installed_at: self.ctx.installed_at.clone(),
        };
        if self.lock.agents.get(name) != Some(&entry) {
            self.lock.agents.insert(name.to_owned(), entry);
            self.lock_touched = true;
        }
    }

    /// Returns whether the removal was accepted, so the caller knows whether
    /// the manifest declaration should follow it.
    fn remove(&mut self, name: &str) -> bool {
        let Some(state) = self.ctx.state_of(name) else {
            self.refuse(
                name,
                AgentState::NotInStore,
                "this project has no agent by that name",
            );
            return false;
        };
        if !state.is_ours_to_touch() {
            self.refuse(
                name,
                state,
                "vibe did not install this file, so it will not delete it",
            );
            return false;
        }

        self.removing = true;
        if self.lock.agents.remove(name).is_some() {
            self.lock_touched = true;
        }

        let rel = self
            .ctx
            .locked
            .get(name)
            .map_or_else(|| install_path(name), |e| e.path.clone());
        let path = self.ctx.project_dir.join(&rel);
        // `Missing` has no file to delete; dropping the entry is the whole job.
        if let Ok(observed) = std::fs::read(&path) {
            self.file_ops.push(FileOp::RemoveOwnedAgent {
                path,
                observed_hash: lock::content_hash(&observed),
            });
        }
        true
    }

    fn declare(&mut self, names: &BTreeSet<String>) {
        self.manifest_installed = Some(names.iter().cloned().collect());
    }

    fn finish(self, intent: PlanIntent) -> Result<AgentPlan, CoreError> {
        let Builder {
            ctx,
            file_ops,
            removing,
            lock,
            lock_touched,
            manifest_installed,
            refused,
        } = self;

        let lock_op = if lock_touched {
            Some(lock::write_op(&ctx.project_dir, &lock)?)
        } else {
            None
        };

        let mut ops: Vec<FileOp> = Vec::new();
        // ADR-0006 §4. `add`: the file lands first, so a crash leaves an
        // unowned file (recoverable by re-running `add`) rather than a lock
        // entry claiming a file we never wrote. `remove`: inverted, so a crash
        // leaves a file with no entry — unowned and untouched — rather than an
        // entry pointing at something gone.
        if removing {
            ops.extend(lock_op);
            ops.extend(file_ops);
        } else {
            ops.extend(file_ops);
            ops.extend(lock_op);
        }

        if let Some(installed) = manifest_installed {
            ops.extend(manifest_op(&ctx.project_dir, installed)?);
        }

        Ok(AgentPlan {
            plan: WritePlan::new(intent, ctx.project_dir.clone(), ops),
            report: ctx.report.clone(),
            refused,
            staleness: ctx.staleness,
        })
    }
}

/// The manifest edit that declares `installed`, plus the version migration it
/// implies.
///
/// A manifest that declares `1.0` and then carries a `1.1` table is lying about
/// itself, and `compat()` is what every other build consults to decide whether
/// to warn. Migrating is a *visible* line in `--dry-run` via
/// [`EditReason::SchemaMigration`], never a silent side effect.
fn manifest_op(project_dir: &Path, installed: Vec<String>) -> Result<Option<FileOp>, CoreError> {
    let file = manifest_path(project_dir);
    let mut doc = ManifestDocument::open(&file)?;
    let manifest = doc.parse()?;

    if manifest.agents.installed == installed {
        return Ok(None);
    }

    let from = manifest.schema_version;
    doc.apply(FieldEdit::ReplaceAgentsInstalled(installed))?;

    // Only ever upward. A manifest already declaring 1.2 knows about things
    // this build does not, and rewriting it to 1.1 would claim it had lost
    // them.
    let migrated = from < crate::SchemaVersion::CURRENT;
    if migrated {
        doc.apply(FieldEdit::SetSchemaVersion(crate::SchemaVersion::CURRENT))?;
    }

    let reason = if migrated {
        EditReason::SchemaMigration {
            from,
            to: crate::SchemaVersion::CURRENT,
        }
    } else {
        EditReason::FieldsUpdated
    };
    Ok(doc.into_op(reason))
}

/// What the manifest declares, or an empty set if there is no manifest.
///
/// A project with no `.vibe/project.toml` declares nothing. That is not an
/// error for a read command — but every write path here goes through
/// [`manifest_op`], which opens the manifest properly and surfaces the real
/// error if it is missing.
fn declared_agents(project_dir: &Path) -> Result<BTreeSet<String>, CoreError> {
    let file = manifest_path(project_dir);
    match ManifestDocument::open(&file) {
        Ok(doc) => Ok(doc.parse()?.agents.installed.into_iter().collect()),
        Err(CoreError::ManifestNotFound { .. }) => Ok(BTreeSet::new()),
        Err(e) => Err(e),
    }
}
