//! The P3 command surface: `list`, `show`, `sync`, `archive`/`unarchive`.
//!
//! These live beside [`crate::Registry`] rather than inside `registry.rs`
//! because they share one rule that is worth stating once, loudly:
//!
//! **An empty field may be filled; a filled field is only ever replaced by
//! another value.**
//!
//! Everything P2 built — `Detected<T>`, `UnknownReason`, the conflict states —
//! collapses into flat manifest fields at this boundary. Without that rule, a
//! field populated last week goes empty this week because `git` happened not to
//! be on `PATH` on this machine. That is data loss wearing a sync's clothes.
//! See the ADR-0003 §6 amendment.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cache::{Cache, CacheEntry, CacheFile, Witness};
use crate::config::{Config, manifest_path};
use crate::error::{CoreError, display_path};
use crate::manifest::{EditReason, FieldEdit, ManifestDocument};
use crate::model::{Detected, UnknownReason};
use crate::plan::{PlanIntent, WritePlan};
use crate::registry::Registry;
use crate::report::Reporter;
use crate::scan::{ScanRequest, ScannedProject};
use crate::view::{ListReport, ProjectSummary, ProjectView, Query};
use crate::walk::discover_projects;

/// What `sync` declined to do, and why.
///
/// The three buckets are not interchangeable, and the split is the same axis as
/// confidence-versus-specificity: all of them block a write, but only `kept` and
/// `unreadable` describe something in the user's project. `not_attempted` is a
/// fact about *this machine* — reporting it as a property of the repository
/// invites the user to go looking for a problem that is not there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct SyncNotes {
    /// Existing values kept because detection had nothing better to offer.
    /// Actionable: the disk disagrees with itself, or a file is malformed.
    pub kept: Vec<String>,
    /// Sources that are present and could not be read.
    pub unreadable: Vec<String>,
    /// Detection could not run at all here. Reported once per run, never per
    /// field, and never as a property of the project.
    pub not_attempted: Vec<String>,
}

impl SyncNotes {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kept.is_empty() && self.unreadable.is_empty() && self.not_attempted.is_empty()
    }
}

impl Registry {
    /// The overview. Cache-backed, and honest about it.
    ///
    /// A cached row is used only when its witness still matches the manifest on
    /// disk. Where cache and disk disagree, **disk wins unconditionally** —
    /// there is no tie-break, because one of them is the user's file and the
    /// other is ours (ADR-0004 §5).
    pub fn list(&self, req: &ScanRequest, query: &Query, rep: &dyn Reporter) -> ListReport {
        let load = self.cache().load();
        let cache_note = load.note();
        let cached = load.usable();

        let mut dirs: Vec<PathBuf> = Vec::new();
        for root in req.roots() {
            dirs.extend(discover_projects(root, req.max_depth()).projects);
        }
        dirs.sort();
        dirs.dedup();

        let mut projects = Vec::new();
        let mut from_cache = 0usize;
        let mut rescan: Vec<PathBuf> = Vec::new();

        for dir in &dirs {
            let key = display_path(dir).0;
            let current = Witness::observe(&manifest_path(dir));
            match cached.entries.get(&key) {
                Some(entry) if entry.witness.still_valid(&current) => {
                    from_cache += 1;
                    projects.push(ProjectSummary::from_cache_entry(entry));
                }
                // A cache miss is a latency cost, never a correctness one.
                _ => rescan.push(dir.clone()),
            }
        }

        if !rescan.is_empty() {
            let mut fresh_req = ScanRequest::default();
            for dir in &rescan {
                fresh_req = fresh_req.with_root(dir.clone());
            }
            // Depth 0: each path already *is* a project directory.
            let fresh = self.scan(&fresh_req.with_max_depth(0), rep);
            for p in &fresh.projects {
                projects.push(summary_from_scan(p));
            }
            self.write_cache(&cached, &fresh.projects);
        }

        projects.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));

        let archived_hidden = projects.iter().filter(|p| !query.matches(p)).count();
        projects.retain(|p| query.matches(p));

        ListReport {
            projects,
            from_cache,
            archived_hidden,
            cache_note,
        }
    }

    /// The detail. **Never cached** — `show` re-reads the manifest every time,
    /// because its whole purpose is to be the thing you paste at an agent, and
    /// a cached answer would undermine exactly that.
    pub fn show(&self, project_dir: &Path) -> Result<ProjectView, CoreError> {
        let project_dir = &crate::registry::absolutize(project_dir)?;
        let manifest_file = manifest_path(project_dir);
        let doc = ManifestDocument::open(&manifest_file)?;
        let manifest = doc.parse()?;
        Ok(ProjectView {
            path: display_path(project_dir).0,
            manifest_path: display_path(&manifest_file).0,
            schema_version: manifest.schema_version,
            compat: manifest.schema_version.compat(),
            unknown_keys: manifest.unknown.clone(),
            manifest,
        })
    }

    /// Re-read the disk and propose manifest updates.
    pub fn plan_sync(
        &self,
        project_dir: &Path,
        rep: &dyn Reporter,
    ) -> Result<(WritePlan, SyncNotes), CoreError> {
        // Absolute, because `plan::validate_path` refuses a relative op path:
        // a plan carries resolved paths so it cannot be applied against an
        // ambient working directory that differs from the one it was built in.
        let project_dir = &crate::registry::absolutize(project_dir)?;
        let manifest_file = manifest_path(project_dir);
        let mut doc = ManifestDocument::open(&manifest_file)?;
        // Parsed before anything is written: a manifest whose major schema this
        // build cannot read must not be edited at all (ADR-0002 §3).
        let manifest = doc.parse()?;

        let scanned = self.scan(&ScanRequest::new(project_dir).with_max_depth(0), rep);
        let Some(found) = scanned.projects.first() else {
            return Ok((
                WritePlan::new(PlanIntent::Sync, project_dir, Vec::new()),
                SyncNotes::default(),
            ));
        };
        let d = &found.detection;

        let mut notes = SyncNotes::default();
        let mut edits: Vec<FieldEdit> = Vec::new();

        let mut scalar = |field: &str, current: Option<&str>, detected: &Detected<String>| {
            consider(field, current, detected, &mut notes)
        };

        if let Some(v) = scalar(
            "stack.runtime",
            manifest.stack.runtime.as_deref(),
            &d.runtime,
        ) {
            edits.push(FieldEdit::SetStackRuntime(v));
        }
        if let Some(v) = scalar("repo.remote", manifest.repo.remote.as_deref(), &d.remote) {
            edits.push(FieldEdit::SetRepoRemote(v));
        }
        if let Some(v) = scalar("deploy.url", manifest.deploy.url.as_deref(), &d.deploy_url) {
            edits.push(FieldEdit::SetDeployUrl(v));
        }
        if let Some(v) = scalar(
            "project.description",
            manifest.project.description.as_deref(),
            &d.description,
        ) {
            edits.push(FieldEdit::SetDescription(Some(v)));
        }

        // Set-valued fields union rather than replace, so a framework the user
        // added by hand is never dropped by a detector that does not know it.
        if let Some(v) = merged(&manifest.stack.frameworks, &d.frameworks) {
            edits.push(FieldEdit::ReplaceStackFrameworks(v));
        }
        if let Some(v) = merged(&manifest.stack.services, &d.services) {
            edits.push(FieldEdit::ReplaceStackServices(v));
        }
        if let Some(v) = merged(&manifest.deploy.env_required, &d.env_required) {
            edits.push(FieldEdit::ReplaceDeployEnvRequired(v));
        }

        for u in &d.unreadable {
            notes.unreadable.push(format!("{} ({})", u.path, u.why));
        }
        notes.kept.sort();
        notes.kept.dedup();
        notes.not_attempted.sort();
        notes.not_attempted.dedup();

        for edit in edits {
            doc.apply(edit)?;
        }
        let ops = doc
            .into_op(EditReason::Synced)
            .map_or_else(Vec::new, |op| vec![op]);
        Ok((WritePlan::new(PlanIntent::Sync, project_dir, ops), notes))
    }

    /// Set or clear the archived flag.
    ///
    /// Never touches `status`, which is what makes un-archiving a restoration
    /// rather than a guess. Both directions are no-ops when already in the
    /// target state — an empty plan, not an error (ADR-0002 §6).
    pub fn plan_archive(&self, project_dir: &Path, archived: bool) -> Result<WritePlan, CoreError> {
        let project_dir = &crate::registry::absolutize(project_dir)?;
        let manifest_file = manifest_path(project_dir);
        let mut doc = ManifestDocument::open(&manifest_file)?;
        let _ = doc.parse()?;
        doc.apply(FieldEdit::SetArchived(archived))?;

        let reason = if archived {
            EditReason::Archived
        } else {
            EditReason::Unarchived
        };
        let ops = doc.into_op(reason).map_or_else(Vec::new, |op| vec![op]);
        Ok(WritePlan::new(PlanIntent::Archive, project_dir, ops))
    }

    fn cache(&self) -> Cache {
        Cache::at(self.cache_path())
    }

    fn write_cache(&self, previous: &CacheFile, fresh: &[ScannedProject]) {
        let mut file = previous.clone();
        file.version = crate::cache::CACHE_VERSION;
        for p in fresh {
            file.entries.insert(
                p.path.clone(),
                CacheEntry {
                    path: p.path.clone(),
                    name: p.name.clone(),
                    status: p.status.clone(),
                    archived: p.archived,
                    runtime: p.detection.runtime.value().cloned(),
                    remote: p.detection.remote.value().cloned(),
                    last_commit: p.detection.last_commit.value().cloned(),
                    deploy_url: p.detection.deploy_url.value().cloned(),
                    witness: Witness::observe(&manifest_path(Path::new(&p.path))),
                },
            );
        }
        // Entries for projects that no longer exist are left rather than
        // evicted. The cache accumulates ghosts, which cost bytes; evicting on
        // absence would let a scan of one root silently delete the registry's
        // memory of another.
        let _ = self.cache().save(&file);
    }
}

/// Decide whether one scalar field may be written, recording why not.
///
/// Returns the value to write, or `None`. The `None` cases are where the rule
/// lives: a populated field is never emptied, whatever the reason detection
/// came back without a value.
fn consider(
    field: &str,
    current: Option<&str>,
    detected: &Detected<String>,
    notes: &mut SyncNotes,
) -> Option<String> {
    if let Some(v) = detected.writable_value() {
        return if current == Some(v.as_str()) {
            None
        } else {
            Some(v.clone())
        };
    }

    // Nothing detected. An *empty* field with nothing to fill it is not worth
    // mentioning; a populated one is being deliberately left alone, and why
    // decides whether the user can do anything about it.
    current?;
    match detected.reason() {
        Some(UnknownReason::NotAttempted { why, .. }) => notes.not_attempted.push(why.clone()),
        Some(UnknownReason::Timeout { detector }) => {
            notes.not_attempted.push(format!("{detector} timed out"));
        }
        // Conflict, LowConfidenceOnly, Unreadable — the user can act on these.
        Some(UnknownReason::Conflict { .. } | UnknownReason::LowConfidenceOnly { .. }) => {
            notes.kept.push(field.to_owned());
        }
        Some(UnknownReason::Unreadable { .. }) => notes.kept.push(field.to_owned()),

        // Nothing on disk spoke to it. Silence is the honest report.
        //
        // No wildcard arm: `UnknownReason` is `#[non_exhaustive]` for
        // downstream crates but exhaustive here, so adding a reason breaks this
        // match and forces someone to decide whether it is actionable. That is
        // the intended friction — a new reason silently defaulting to "say
        // nothing" is how honest reporting erodes.
        Some(UnknownReason::NoEvidence) | None => {}
    }
    None
}

/// The union of what is recorded and what was detected, or `None` if unchanged.
fn merged(current: &[String], detected: &[String]) -> Option<Vec<String>> {
    if detected.is_empty() {
        return None;
    }
    let mut out: Vec<String> = current.to_vec();
    out.extend(detected.iter().cloned());
    out.sort();
    out.dedup();
    if out == current { None } else { Some(out) }
}

fn summary_from_scan(p: &ScannedProject) -> ProjectSummary {
    ProjectSummary {
        path: p.path.clone(),
        name: p.name.clone(),
        status: p.status.clone(),
        archived: p.archived,
        runtime: p.detection.runtime.value().cloned(),
        remote: p.detection.remote.value().cloned(),
        last_commit: p.detection.last_commit.value().cloned(),
        deploy_url: p.detection.deploy_url.value().cloned(),
        from_cache: false,
        error: p.manifest_error.clone(),
    }
}

/// Where the cache lives, if anywhere.
pub(crate) fn default_cache_path() -> Option<PathBuf> {
    Config::cache_dir().map(|d| d.join(crate::cache::cache_file_name()))
}
