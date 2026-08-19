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

/// The temporary path one [`write_atomically`] will use, beside its target.
///
/// **Exposed so the naming property can be asserted without a race.** The first
/// version of its control watched the directory from a spinning thread and
/// asserted it had seen more than one `.tmp` name — a control whose firing
/// depends on the reader being scheduled at the right moment, which is exactly
/// what ADR-0002 §7 refuses because it can stop proving anything without ever
/// failing. It was caught doing that once, in the round it was written.
///
/// Same seam as `vibectl`'s `panic_report`: the part that can be asserted
/// directly is extracted, so the assertion does not have to be inferred from
/// timing.
///
/// **Each call advances the serial**, so two calls never agree — which is the
/// property, and is why this is not a pure function of its argument.
#[must_use]
pub fn temp_path_for(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map_or_else(|| "vibe".to_owned(), |n| n.to_string_lossy().into_owned());
    let serial = TEMP_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dir.join(format!(".{name}.vibe-{}-{serial}.tmp", std::process::id()))
}

/// Serial number for temp names, so two writes in one process cannot collide.
static TEMP_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Replace a file's contents without ever leaving it partial.
///
/// # The defect this repairs, and where it lived
///
/// *Added 2026-08-19.* `std::fs::write` is `File::create` followed by
/// `write_all`, and `File::create` **truncates before any byte is written**.
/// There was therefore a window where the target is **zero bytes**, and it
/// existed **by construction** — not a race that might not happen, a state the
/// sequence passed through every single time.
///
/// **This was not install's path.** It is the arm of [`apply`] that serves
/// `CreateFile` *and* `UpdateFile`, which is every manifest write this tool has
/// ever done: `vibe new`, `sync`, `archive`, `render`. See ADR-0001's defect
/// entry for the cost.
///
/// # The shape
///
/// Write to a temporary file **beside the target**, then rename over it. A
/// reader sees the old file or the new one.
///
/// **Beside the target, not in the system temp directory**, and that is
/// load-bearing rather than tidy: a rename across volumes is a copy plus a
/// delete, which puts the window back and adds a delete to a tool that has
/// none. The temp path is derived from the target, so it inherits the
/// containment already checked for it (ADR-0005 §10 rule 5).
///
/// **The name carries the process id and a serial**, so neither two vibe
/// processes nor two writes inside one process can land on one temp. What it
/// does not do is coordinate two writers of the same target: they still race on
/// the rename, and one of them wins **whole**. That is the property here, and
/// extending it to mutual exclusion would be a lock, which this project does
/// not have.
///
/// # What is measured, and what is only known
///
/// **Measured** (ADR-0011 §2 round 3f, and in the ordinary test job on all three
/// platforms): a reader spinning on the target through 400 replacements sees
/// only whole contents, where the same reader catches `std::fs::write` at
/// `Empty`. **The zero is bounded by that reader's resolution** — the truncating
/// window is long and a rename's is short, so *"never observed"* is weaker than
/// *"cannot occur"*, and the structural argument below is what carries the
/// claim.
///
/// **Structural.** On POSIX, `rename(2)` is specified atomic: a reader sees the
/// old inode or the new one. On Windows, `std::fs::rename` calls `MoveFileExW`
/// with `MOVEFILE_REPLACE_EXISTING`, which replaces an existing destination —
/// and **fails, leaving the destination untouched, when another process holds it
/// open without `FILE_SHARE_DELETE`**. Measured on Windows 10 Pro 19045: a
/// holder at `FileShare.None` or `FileShare.Read` refuses the replacement and
/// the original survives; a holder at `FileShare.ReadWrite | Delete`, which is
/// what Rust's own `File::open` requests, permits it. **The failure direction is
/// the safe one** — an error, with the user's file intact.
///
/// # What it does NOT promise
///
/// **Durability is not promised, and nothing here claims what a power failure
/// cannot produce.** There is no `fsync` of the temp file and none of the
/// directory, so a crash can lose the new contents. Whether it can also lose the
/// *old* ones is a property of the filesystem, not of this code — ext4 with
/// delayed allocation historically could, before the rename heuristics — and it
/// is **not measurable here**, so it is not asserted. The strong version costs
/// two syncs on a path the manifest write takes constantly, and it has not been
/// costed.
///
/// **Permissions are carried, and only the ones the standard library models.**
/// When the target exists, the temp is **created already carrying** its
/// permissions — not corrected afterwards, which would leave the contents of a
/// `0600` file on disk at the umask for a moment. See [`write_temp`] for the
/// per-platform difference and for the **ACL direction, which is widening**.
///
/// # Errors
///
/// The io error, with the path it happened on. A failure to write the temp
/// leaves the target untouched; a failure to rename leaves the target untouched
/// **and the temp file behind**, which is reported rather than swallowed —
/// removing it would need a delete on the error path of a tool that deliberately
/// has none, and a stray `.settings.json.vibe-1234-7.tmp` is a visible fact
/// about a failed write rather than a silent one.
pub fn write_atomically(path: &Path, contents: &str) -> Result<(), CoreError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map_or_else(|| "vibe".to_owned(), |n| n.to_string_lossy().into_owned());
    let serial = TEMP_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = dir.join(format!(".{name}.vibe-{}-{serial}.tmp", std::process::id()));

    let io = |p: &Path, source: std::io::Error| CoreError::Io {
        path: p.to_path_buf(),
        source,
    };

    write_temp(&temp, path, contents).map_err(|e| io(&temp, e))?;

    if let Err(e) = std::fs::rename(&temp, path) {
        // Remove the temp WE just created, at a path derived moments ago and
        // carrying this process's id and serial. A refused rename is not rare
        // the way a kill is — a Windows holder without `FILE_SHARE_DELETE`
        // refuses it every time, so ten retried installs would leave ten temp
        // files inside `.claude/`, a directory another tool reads. That is
        // accumulation, and the "a visible stray file beats a silent one"
        // argument was made for the KILL case, where no error path runs at all
        // and the residue is unavoidable. It still applies there, and it does
        // not apply here.
        //
        // This is a delete in a tool whose second constraint is about not
        // deleting, so it is the `FileOp::RemoveOwnedAgent` argument at one
        // remove: the path is not user-supplied, the file did not exist before
        // this call, and nothing else can have written it. The failure to
        // remove is ignored because the rename error is the one worth
        // reporting.
        let _ = std::fs::remove_file(&temp);
        return Err(io(path, e));
    }
    Ok(())
}

/// Create the temp file **already carrying the target's permissions**, rather
/// than correcting them afterwards.
///
/// *Split out 2026-08-19.* The first version wrote the contents and then called
/// `set_permissions`, which leaves a window where the bytes of a `0600`
/// `settings.json` are on disk at whatever the umask allows. **That is the same
/// window class this whole primitive exists to close**, one layer in, and it
/// was reintroduced by the fix for a different problem.
///
/// On **Unix** the mode is applied at `open` time, so no window exists. On
/// **Windows** the only bit `std::fs::Permissions` models is read-only, which
/// carries no exposure — a file readable a moment early is not a leak when the
/// flag never controlled who could read it — so it is set after the write, and
/// that difference is stated rather than hidden behind one code path.
///
/// **ACLs are not carried, and the direction is widening.** On Windows the
/// renamed file keeps the temp's ACL, which inherits from the directory. If the
/// original had a *narrower* ACL than the directory grants, the replacement is
/// **more** accessible than what it replaced. That is the less safe direction,
/// and it is recorded as such rather than as *"ACLs are not carried"*.
fn write_temp(temp: &Path, target: &Path, contents: &str) -> std::io::Result<()> {
    let existing = std::fs::metadata(target).ok().map(|m| m.permissions());

    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        if let Some(perms) = &existing {
            opts.mode(perms.mode());
        }
        let mut file = opts.open(temp)?;
        file.write_all(contents.as_bytes())?;
        return Ok(());
    }

    #[cfg(not(unix))]
    {
        std::fs::write(temp, contents)?;
        if let Some(perms) = existing {
            std::fs::set_permissions(temp, perms)?;
        }
        Ok(())
    }
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
