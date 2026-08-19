//! The plan/apply mutation model.
//!
//! Nothing in this crate writes to the filesystem except [`apply`], and the
//! only thing it accepts is a [`WritePlan`]. `--dry-run` is therefore not a
//! flag anything here knows about — it is the caller building a plan and
//! declining to apply it. A write path that forgot to honour `--dry-run` is
//! not possible, because there is no write path that does not go through a
//! plan.
//!
//! # What is deliberately absent
//!
//! - **There is no `FileOp::Delete`.** The "never destructive" constraint is
//!   enforced by the variant not existing, so a destructive command is
//!   unrepresentable rather than merely discouraged.
//!
//!   [`FileOp::RemoveOwnedAgent`] is not that variant, and the distinction is
//!   worth being precise about rather than waving at. A general `Delete` takes
//!   a path and removes whatever is there; `RemoveOwnedAgent` can only express
//!   "delete this file, whose content hashes to exactly this" — and the op is
//!   only ever *built* for a path the ADR-0006 §5 state table has already said
//!   is ours. `vibe agents remove` needs to delete a file it installed; it does
//!   not need, and cannot express, deleting anything else. The dangerous
//!   capability stays unrepresentable.
//! - **[`WritePlan`] is `Serialize` but not `Deserialize`.** A plan contains
//!   file contents and (from P5) subprocess invocations. Deserialising one
//!   would mean `apply` accepts arbitrary file writes and arbitrary process
//!   execution as *data* — from a webview, in a desktop build. Nothing in v1
//!   needs the inverse direction, and the derive is far cheaper to withhold now
//!   than to remove later (ADR-0005 §1).
//! - **There is no `RunCommand` op yet.** It arrives in P5 shaped as a closed
//!   set of validated git operations, not `{ program, args }` — a `{git, gh}`
//!   program allowlist over free-form argv is not a containment boundary
//!   (ADR-0005 §10).

use std::path::{Component, Path, PathBuf};

use serde::{Serialize, Serializer};

use crate::error::{CoreError, display_path};
use crate::manifest::EditReason;
use crate::report::{Event, Reporter};

/// Serialise a path as a lossy string rather than failing.
///
/// `serde`'s own `PathBuf` impl *errors* on non-UTF-8, which would make
/// `--json` fail outright on a path it could still usefully describe. The
/// sibling `path_lossy` flag that ADR-0005 §4 specifies is emitted by
/// [`crate::ErrorPayload`] today; wiring it through every plan op arrives with
/// the full `--json` DTOs in P3.
fn lossy_path<S: Serializer>(p: &Path, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&display_path(p).0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PlanIntent {
    New,
    Sync,
    Render,
    Archive,
    AgentsAdd,
    AgentsRemove,
    AgentsSync,
}

/// A single filesystem change. Additive only, by construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
#[non_exhaustive]
pub enum FileOp {
    CreateDir {
        #[serde(serialize_with = "lossy_path")]
        path: PathBuf,
    },
    CreateFile {
        #[serde(serialize_with = "lossy_path")]
        path: PathBuf,
        contents: String,
    },
    /// Carries both sides so a caller can render a real diff without re-reading
    /// the file, and so `apply` can verify the file has not moved underneath
    /// the plan.
    UpdateFile {
        #[serde(serialize_with = "lossy_path")]
        path: PathBuf,
        before: String,
        after: String,
        reason: EditReason,
    },
    /// Delete an agent file this tool installed.
    ///
    /// **This is not `FileOp::Delete`, and the difference is the whole point.**
    /// The module docs say a destructive command is unrepresentable rather than
    /// discouraged, and that stays true: there is still no op that deletes an
    /// arbitrary path. `vibe agents remove` needs to delete *one specific kind
    /// of file* — one we wrote, that is still exactly as we wrote it — so it
    /// gets a variant that can express only that, and carries the proof.
    ///
    /// `observed_hash` is the content the plan saw, not the content the
    /// lockfile recorded. The two differ for a `Modified` agent, which
    /// ADR-0006 §5 says `remove` still deletes because it is ours; what the
    /// hash guards is the window between planning and applying. It is the same
    /// contract as [`FileOp::UpdateFile`]'s `before`: if the file is not what
    /// the plan diffed against, the world moved and the plan is stale.
    ///
    /// Ownership itself is decided *before* the op is built, by the state
    /// table. An op reaching `apply` has already been through
    /// [`crate::AgentState::is_ours_to_touch`].
    RemoveOwnedAgent {
        #[serde(serialize_with = "lossy_path")]
        path: PathBuf,
        observed_hash: String,
    },
}

impl FileOp {
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            FileOp::CreateDir { path }
            | FileOp::CreateFile { path, .. }
            | FileOp::UpdateFile { path, .. }
            | FileOp::RemoveOwnedAgent { path, .. } => path,
        }
    }
}

/// Everything a write command intends to do, and nothing it has done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
#[must_use]
pub struct WritePlan {
    pub intent: PlanIntent,
    /// Every op must resolve inside this directory. Recorded on the plan rather
    /// than passed to `apply` separately so a plan carries its own containment
    /// boundary and cannot be applied against a wider one.
    #[serde(serialize_with = "lossy_path")]
    pub root: PathBuf,
    pub ops: Vec<FileOp>,
}

impl WritePlan {
    pub fn new(intent: PlanIntent, root: impl Into<PathBuf>, ops: Vec<FileOp>) -> Self {
        Self {
            intent,
            root: root.into(),
            ops,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
#[non_exhaustive]
pub enum ApplyOutcome {
    Completed,
    /// The caller cancelled. A *success* outcome: work already done stands, and
    /// `after_ops` lets a consumer say "created 3 of 7 files, re-run to finish"
    /// rather than showing an error dialog (ADR-0005 §2).
    Cancelled {
        after_ops: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ApplyReport {
    pub applied: Vec<String>,
    pub skipped: Vec<SkippedOp>,
    pub outcome: ApplyOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct SkippedOp {
    pub path: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SkipReason {
    /// A `CreateDir` whose directory already exists. Creating a directory is
    /// idempotent; this is not a failure.
    DirectoryAlreadyExists,
    /// A `RemoveOwnedAgent` whose file was gone by the time it ran. The
    /// intended end state holds.
    AlreadyAbsent,
}

/// Verify one op path against the plan's root and the containment rules.
///
/// Rules, in the order they are cheapest to check:
///
/// 1. **Absolute.** Plans carry resolved paths; a relative one means the plan
///    was built against an ambient working directory that `apply` may not share.
/// 2. **No `..` or `.` components.** Normalisation happens at plan time.
/// 3. **No `.git` component, ever.** `.git/hooks/*` executes with no config and
///    no argument, so an additive write there is code execution — this is what
///    makes "the worst case is an unintended additive write" true rather than
///    wishful (ADR-0005 §10 rule 6).
/// 4. **Lexically inside `root`.**
/// 5. **The deepest existing ancestor canonicalises to somewhere inside the
///    canonicalised root.** Catches a symlinked intermediate directory that
///    exists at check time.
///
/// Rule 5 is *not* TOCTOU-proof: between this check and the write, a parent can
/// be swapped for a symlink or a directory junction. Closing that needs
/// `openat`-style handle-relative I/O, which v1 does not attempt. Rule 3 is
/// what bounds the consequence.
pub fn validate_path(path: &Path, root: &Path) -> Result<(), CoreError> {
    let escapes = || CoreError::PathEscapesRoot {
        path: path.to_path_buf(),
        root: root.to_path_buf(),
    };

    if !path.is_absolute() || !root.is_absolute() {
        return Err(escapes());
    }

    for comp in path.components() {
        match comp {
            Component::ParentDir | Component::CurDir => return Err(escapes()),
            Component::Normal(name) if name.eq_ignore_ascii_case(".git") => {
                return Err(CoreError::PathInsideGitDir {
                    path: path.to_path_buf(),
                });
            }
            _ => {}
        }
    }

    if !path.starts_with(root) {
        return Err(escapes());
    }

    let real_root = canonical_existing_ancestor(root);
    let real_path = canonical_existing_ancestor(path);
    match (real_root, real_path) {
        (Some(r), Some(p)) if !p.starts_with(&r) => Err(escapes()),
        _ => Ok(()),
    }
}

/// Canonicalise the deepest ancestor that exists.
///
/// `canonicalize` fails on a path that does not exist yet, which is every path
/// a `CreateFile` names. Walking up to the first existing ancestor is what lets
/// the check run before the write rather than after it.
fn canonical_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut cur = Some(path);
    while let Some(p) = cur {
        if let Ok(real) = p.canonicalize() {
            return Some(real);
        }
        cur = p.parent();
    }
    None
}

/// Replace a file's contents without ever leaving it partial.
///
/// # Why this exists, and why `std::fs::write` was not enough
///
/// *Added 2026-08-19.* `std::fs::write` is `File::create` followed by
/// `write_all`, and `File::create` **truncates before any byte is written**.
/// There is therefore a window where the target is **zero bytes**, and unlike
/// the kernel window ADR-0011 §2 sampled thirty-five times without finding, this
/// one exists **by construction** — it is not a race that might not happen, it
/// is a state the sequence passes through every single time.
///
/// Three rounds of ADR-0011 went into whether a killed hook could tear a record
/// in vibe's own sink, where the writer appends, the reader tolerates damage,
/// and every whole record before the damage survives. **None of that transfers
/// here.** This path rewrites files whose readers have no tolerance at all —
/// `.claude/settings.json` is read by a strict JSON loader, and it is a file
/// vibe does not own. Hard constraint 2 is not *"there is no
/// `FileOp::Delete`"*; the absent variant is how the constraint is enforced,
/// and the constraint is **never destructive**. A zero-byte `settings.json` is
/// destructive by any reading of that sentence.
///
/// # The shape
///
/// Write the new contents to a temporary file **beside the target**, then
/// rename it over. A reader either sees the old file or the new one.
///
/// **Beside the target, not in the system temp directory**, and that is
/// load-bearing rather than tidy: a rename across volumes is a copy plus a
/// delete, which puts the window back and adds a delete to a tool that has
/// none. The temp path is derived from the target, so it inherits the
/// containment already checked for it (ADR-0005 §10 rule 5) — it cannot name a
/// directory the target could not.
///
/// **Measured rather than read off documentation**, because *"rename is
/// atomic"* is exactly the class of cross-platform claim that died on contact
/// with measurement in ADR-0002 §7: see `a_replace_is_never_observed_partial`,
/// which spin-reads a target through many replacements and reports every
/// distinct state it sees — **paired against `std::fs::write`, which the same
/// instrument catches mid-truncation.** Without that pairing a clean result
/// would only say the reader was too slow.
///
/// # What it does not promise
///
/// Not durability. There is no `fsync` here, so a power failure can still lose
/// the new contents — what it cannot do is leave the target empty or half
/// written. Durability is a different property with a different cost, and
/// claiming it would be the label reaching past the mechanism again.
///
/// # Errors
///
/// The io error, with the path it happened on. A failure to write the temp file
/// leaves the target untouched; a failure to rename leaves the target untouched
/// and the temp file behind, which is reported rather than swallowed.
pub fn write_atomically(path: &Path, contents: &str) -> Result<(), CoreError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map_or_else(|| "vibe".to_owned(), |n| n.to_string_lossy().into_owned());

    // The temp name carries the pid so two vibe processes writing the same
    // target cannot collide on it. They still race on the rename, and the
    // loser's contents win or lose whole — which is the property this function
    // is for, not one it can extend to coordinating two writers.
    let temp = dir.join(format!(".{name}.vibe-{}.tmp", std::process::id()));

    let io = |p: &Path, source: std::io::Error| CoreError::Io {
        path: p.to_path_buf(),
        source,
    };

    std::fs::write(&temp, contents).map_err(|e| io(&temp, e))?;

    if let Err(e) = std::fs::rename(&temp, path) {
        // Leave the temp file. Removing it here would need a delete on the
        // error path of a tool that deliberately has none, and a stray
        // `.settings.json.vibe-1234.tmp` next to the file is a visible fact
        // about a failed write rather than a silent one.
        return Err(io(path, e));
    }
    Ok(())
}

/// Execute a plan.
///
/// Every op is validated and every precondition checked **before any op runs**,
/// so a plan that cannot complete fails without having written anything. It is
/// not transactional at the filesystem level: a crash midway leaves a partial —
/// but always additive — result, and re-running re-plans against the new
/// reality (ADR-0001, Consequences).
pub fn apply(plan: &WritePlan, rep: &dyn Reporter) -> Result<ApplyReport, CoreError> {
    let started = std::time::Instant::now();

    for op in &plan.ops {
        validate_path(op.path(), &plan.root)?;
        check_precondition(op)?;
    }

    rep.event(Event::ApplyStarted {
        ops: plan.ops.len(),
    });

    let mut applied = Vec::new();
    let mut skipped = Vec::new();
    let mut outcome = ApplyOutcome::Completed;

    for (index, op) in plan.ops.iter().enumerate() {
        if rep.should_cancel() {
            outcome = ApplyOutcome::Cancelled { after_ops: index };
            break;
        }
        let display = display_path(op.path()).0;
        match op {
            FileOp::CreateDir { path } => {
                if path.is_dir() {
                    skipped.push(SkippedOp {
                        path: display,
                        reason: SkipReason::DirectoryAlreadyExists,
                    });
                    continue;
                }
                std::fs::create_dir_all(path).map_err(|source| CoreError::Io {
                    path: path.clone(),
                    source,
                })?;
                applied.push(display.clone());
            }
            FileOp::CreateFile { path, contents }
            | FileOp::UpdateFile {
                path,
                after: contents,
                ..
            } => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|source| CoreError::Io {
                        path: parent.to_path_buf(),
                        source,
                    })?;
                }
                write_atomically(path, contents)?;
                applied.push(display.clone());
            }
            FileOp::RemoveOwnedAgent { path, .. } => {
                match std::fs::remove_file(path) {
                    Ok(()) => applied.push(display.clone()),
                    // Already gone between the precondition check and here.
                    // The intended end state holds, so this is not a failure —
                    // and a re-run after a crash must not fail on the step that
                    // already succeeded.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        skipped.push(SkippedOp {
                            path: display.clone(),
                            reason: SkipReason::AlreadyAbsent,
                        });
                    }
                    Err(source) => {
                        return Err(CoreError::Io {
                            path: path.clone(),
                            source,
                        });
                    }
                }
            }
        }
        rep.event(Event::OpApplied { path: display });
    }

    rep.event(Event::ApplyFinished {
        applied: applied.len(),
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    });

    Ok(ApplyReport {
        applied,
        skipped,
        outcome,
    })
}

fn check_precondition(op: &FileOp) -> Result<(), CoreError> {
    match op {
        FileOp::CreateDir { .. } => Ok(()),
        FileOp::CreateFile { path, .. } => {
            if path.exists() {
                Err(CoreError::TargetExists { path: path.clone() })
            } else {
                Ok(())
            }
        }
        // The file must still hold exactly what the plan diffed against.
        // Anything else means the world moved after planning, and applying
        // `after` would silently discard whatever changed it.
        FileOp::UpdateFile { path, before, .. } => match std::fs::read_to_string(path) {
            Ok(current) if &current == before => Ok(()),
            Ok(_) => Err(CoreError::PlanStale { path: path.clone() }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(CoreError::ManifestNotFound { path: path.clone() })
            }
            Err(source) => Err(CoreError::Io {
                path: path.clone(),
                source,
            }),
        },
        // The same staleness contract as `UpdateFile`, by content hash rather
        // than by full text: a deletion has no `after` to diff against, and the
        // file being deleted is one whose bytes we already had to hash to
        // decide ownership. A file that changed after planning is not the file
        // the user agreed to delete.
        FileOp::RemoveOwnedAgent {
            path,
            observed_hash,
        } => match std::fs::read(path) {
            Ok(current) if &crate::agents::content_hash(&current) == observed_hash => Ok(()),
            Ok(_) => Err(CoreError::PlanStale { path: path.clone() }),
            // Already gone. Deleting is idempotent in the direction that
            // matters, and erroring here would make a re-run after a crash fail
            // on the step that already succeeded.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(CoreError::Io {
                path: path.clone(),
                source,
            }),
        },
    }
}
