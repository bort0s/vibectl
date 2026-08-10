//! `vibe scan` — index what is already on disk.
//!
//! # Scanning structurally cannot write
//!
//! Note the return type of every function here: [`ScanReport`], never
//! [`crate::WritePlan`]. The only function in this crate that touches the
//! filesystem for writing is [`crate::plan::apply`], it takes a `WritePlan`,
//! and nothing on this path constructs one. Turning scan results into manifests
//! is a separate, explicit step that a caller has to ask for — it is not
//! something a scan can do as a side effect.
//!
//! That is a structural guarantee rather than a promise: to make `scan` write,
//! you would have to add a `WritePlan` constructor to this module and a call to
//! `apply`, both of which are visible in a diff.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config::manifest_path;
use crate::detect::{DetectCtx, Detection, ReadCache, builtin_detectors};
use crate::error::{ErrorPayload, display_path};
use crate::exec::ProcessRunner;
use crate::manifest::ManifestDocument;
use crate::report::{Event, Reporter};
use crate::walk::{DEFAULT_DISCOVERY_DEPTH, FileIndex, discover_projects};

/// Input to [`crate::Registry::scan`].
#[derive(Debug, Clone)]
pub struct ScanRequest {
    roots: Vec<PathBuf>,
    max_depth: usize,
    per_project_budget: Option<Duration>,
}

impl Default for ScanRequest {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            max_depth: DEFAULT_DISCOVERY_DEPTH,
            // Generous per project; the point is that one wedged repository
            // cannot consume the whole scan, not to police normal work.
            per_project_budget: Some(Duration::from_millis(3000)),
        }
    }
}

impl ScanRequest {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            roots: vec![root.into()],
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.roots.push(root.into());
        self
    }

    #[must_use]
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    #[must_use]
    pub fn with_per_project_budget(mut self, budget: Option<Duration>) -> Self {
        self.per_project_budget = budget;
        self
    }

    #[must_use]
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
#[non_exhaustive]
pub enum ScanOutcome {
    Completed,
    /// The caller cancelled. A success: the projects already found stand.
    Cancelled {
        after_projects: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ScannedProject {
    pub path: String,
    /// The directory name, or the manifest's `project.name` if one is readable.
    pub name: String,
    pub has_manifest: bool,
    /// Set when a manifest exists but could not be read — a major schema
    /// mismatch, a syntax error, a missing name. The project still appears;
    /// it is an error row, not a missing entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_error: Option<ErrorPayload>,
    pub detection: Detection,
    /// True when the file index hit its cap, so absence is not evidence.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub index_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ScanReport {
    pub roots: Vec<String>,
    pub projects: Vec<ScannedProject>,
    pub outcome: ScanOutcome,
    pub elapsed_ms: u64,
    /// Directories the walk stopped at because `--depth` was reached, and
    /// which still contained subdirectories.
    ///
    /// A project below one of these was not found, and its absence from
    /// `projects` says nothing about whether it exists. Reporting "no projects
    /// found" after declining to look would be the same substitution the
    /// detectors are forbidden from making.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub depth_limited: Vec<String>,
}

impl ScanReport {
    /// Projects whose manifest exists but could not be read.
    #[must_use]
    pub fn unreadable(&self) -> usize {
        self.projects
            .iter()
            .filter(|p| p.manifest_error.is_some())
            .count()
    }

    #[must_use]
    pub fn suggestion_count(&self) -> usize {
        self.projects
            .iter()
            .map(|p| p.detection.suggestions.len())
            .sum()
    }
}

pub(crate) fn scan(req: &ScanRequest, exec: &dyn ProcessRunner, rep: &dyn Reporter) -> ScanReport {
    let started = Instant::now();
    let detectors = builtin_detectors();

    let mut roots: Vec<PathBuf> = Vec::new();
    for root in &req.roots {
        roots.push(root.clone());
    }

    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut depth_limited: Vec<String> = Vec::new();
    for root in &roots {
        let found = discover_projects(root, req.max_depth);
        dirs.extend(found.projects);
        depth_limited.extend(found.depth_limited.iter().map(|p| display_path(p).0));
    }
    dirs.sort();
    dirs.dedup();
    depth_limited.sort();
    depth_limited.dedup();

    rep.event(Event::ScanStarted {
        projects: dirs.len(),
    });

    // Projects are scanned in parallel because the work is dominated by
    // subprocess latency, not by CPU: measured on a 4-core desktop, walking and
    // detecting 50 repositories takes ~93ms while the `git` invocations for the
    // same 50 take ~3.4s. Spawning a process costs ~17ms on Windows and there
    // is nothing to optimise about that except overlapping it.
    //
    // `std::thread::scope` rather than rayon: the pool is bounded, the work
    // items are coarse, and this avoids a dependency for a fixed-size fan-out.
    //
    // Work is pulled from a shared cursor rather than split into static chunks.
    // Static chunking was measured and it is not safe here: four heavy
    // repositories among fifty trivial ones cost a 747ms median when they
    // landed in different chunks and 1177ms (max 1936ms, against a 2000ms
    // budget) when they landed in the same one. Nothing differed but their
    // names.
    //
    // And clustering is the *likely* arrangement, not the unlucky one, because
    // `dirs` is sorted by path and related projects share a prefix — a client's
    // `acme-api`, `acme-web`, `acme-worker` sort adjacently and tend to be
    // similar in size. A cursor makes the distribution irrelevant: a thread
    // that draws a monorepo simply takes fewer items.
    let threads = std::thread::available_parallelism()
        .map_or(4, std::num::NonZeroUsize::get)
        .clamp(1, 8);

    let cursor = std::sync::atomic::AtomicUsize::new(0);
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let mut collected: Vec<(usize, ScannedProject)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let detectors = &detectors;
                let cursor = &cursor;
                let cancelled = &cancelled;
                let dirs = &dirs;
                scope.spawn(move || {
                    let mut out = Vec::new();
                    loop {
                        let index = cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(dir) = dirs.get(index) else { break };
                        if rep.should_cancel() {
                            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                            break;
                        }
                        let scanned = scan_one(dir, detectors, exec, req.per_project_budget);
                        rep.event(Event::ProjectScanned {
                            path: scanned.path.clone(),
                        });
                        out.push((index, scanned));
                    }
                    out
                })
            })
            .collect();

        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .flatten()
            .collect()
    });

    // Deterministic order, independent of thread scheduling.
    collected.sort_by_key(|(index, _)| *index);
    let projects: Vec<ScannedProject> = collected.into_iter().map(|(_, p)| p).collect();

    let outcome = if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        ScanOutcome::Cancelled {
            after_projects: projects.len(),
        }
    } else {
        ScanOutcome::Completed
    };

    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    rep.event(Event::ScanFinished {
        projects: projects.len(),
        elapsed_ms,
    });

    ScanReport {
        roots: roots.iter().map(|r| display_path(r).0).collect(),
        projects,
        outcome,
        elapsed_ms,
        depth_limited,
    }
}

fn scan_one(
    dir: &Path,
    detectors: &[Box<dyn crate::detect::Detector>],
    exec: &dyn ProcessRunner,
    budget: Option<Duration>,
) -> ScannedProject {
    let index = FileIndex::build(dir);
    let reads = ReadCache::new(dir);
    let deadline = budget.map(|b| Instant::now() + b);
    let ctx = DetectCtx::new(dir, &index, &reads, exec, deadline);

    let (findings, failures) = crate::detect::run_detectors(detectors, &ctx);
    let detection = crate::detect::merge(&findings, &failures);

    let manifest_file = manifest_path(dir);
    let (has_manifest, name, manifest_error) = if manifest_file.is_file() {
        match ManifestDocument::open(&manifest_file).and_then(|d| d.parse()) {
            Ok(m) => (true, m.project.name, None),
            Err(e) => (true, dir_name(dir), Some(e.to_wire())),
        }
    } else {
        (false, dir_name(dir), None)
    };

    ScannedProject {
        path: display_path(dir).0,
        name,
        has_manifest,
        manifest_error,
        index_truncated: index.is_truncated(),
        detection,
    }
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| display_path(dir).0)
}
