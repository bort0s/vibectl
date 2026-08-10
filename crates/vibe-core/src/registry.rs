//! The top-level handle.
//!
//! A handle rather than free functions, so a call site does not re-thread the
//! roots list, the cache path and the clock through every call — and so tests
//! can inject all three instead of reaching for global state.
//!
//! Every method takes `&self`. The type is `Send + Sync + 'static` (asserted at
//! compile time in `lib.rs`) so a desktop consumer can put it in managed state
//! and reach it from concurrent command handlers.

use std::path::{Path, PathBuf};

use crate::config::{Config, manifest_path};
use crate::error::CoreError;
use crate::manifest::ManifestDocument;
use crate::model::Status;
use crate::plan::{self, ApplyReport, FileOp, PlanIntent, WritePlan};
use crate::report::Reporter;

/// Input to [`Registry::plan_new`].
///
/// Caller-constructed, so it gets `Default` + setters rather than
/// `#[non_exhaustive]`: a future field must not break existing literals.
#[derive(Debug, Clone)]
pub struct NewRequest {
    name: String,
    parent_dir: PathBuf,
    description: Option<String>,
    status: Status,
}

impl Default for NewRequest {
    fn default() -> Self {
        Self {
            name: String::new(),
            parent_dir: PathBuf::from("."),
            description: None,
            // A scaffolded project is one you are starting work on now. `idea`
            // is for something you have written down but not begun.
            status: Status::Active,
        }
    }
}

impl NewRequest {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_parent_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.parent_dir = dir.into();
        self
    }

    #[must_use]
    pub fn with_description(mut self, description: Option<String>) -> Self {
        self.description = description;
        self
    }

    #[must_use]
    pub fn with_status(mut self, status: Status) -> Self {
        self.status = status;
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone)]
pub struct Registry {
    config: Config,
    exec: std::sync::Arc<dyn crate::exec::ProcessRunner>,
    /// `None` disables caching entirely, which is what tests use so they never
    /// touch the developer's real cache — and is also the honest code path for
    /// a platform with no cache directory.
    cache_path: Option<PathBuf>,
}

impl Registry {
    #[must_use]
    pub fn open(config: Config) -> Self {
        Self {
            config,
            exec: std::sync::Arc::new(crate::exec::SystemRunner::default()),
            cache_path: crate::ops::default_cache_path(),
        }
    }

    /// Point the cache somewhere else, or nowhere.
    #[must_use]
    pub fn with_cache_path(mut self, path: Option<PathBuf>) -> Self {
        self.cache_path = path;
        self
    }

    #[must_use]
    pub(crate) fn cache_path(&self) -> Option<PathBuf> {
        self.cache_path.clone()
    }

    /// Inject a process runner. Tests use this to run without `git`, which is
    /// also the code path a user without `git` installed takes.
    #[must_use]
    pub fn with_runner(mut self, exec: std::sync::Arc<dyn crate::exec::ProcessRunner>) -> Self {
        self.exec = exec;
        self
    }

    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Index what already exists on disk.
    ///
    /// Returns a [`ScanReport`] and nothing else. There is no code path from
    /// here to a [`WritePlan`], so a scan structurally cannot modify anything —
    /// see the `scan` module docs.
    pub fn scan(&self, req: &crate::scan::ScanRequest, rep: &dyn Reporter) -> crate::ScanReport {
        crate::scan::scan(req, self.exec.as_ref(), rep)
    }

    /// Plan a new project directory and its manifest.
    ///
    /// Scaffolding is deliberately thin: a directory and a
    /// `.vibe/project.toml`. This is a registry with scaffolding attached, not
    /// a scaffolder — `cookiecutter` and `npm create` already exist and are
    /// better at it. Git initialisation and repository creation arrive in P5.
    pub fn plan_new(&self, req: &NewRequest) -> Result<WritePlan, CoreError> {
        let name = req.name.trim();
        if name.is_empty() {
            return Err(CoreError::ManifestMissingField {
                path: req.parent_dir.clone(),
                field: "project.name".to_owned(),
            });
        }

        let parent = absolutize(&req.parent_dir)?;
        let project_dir = parent.join(name);

        // Refuse rather than merge into whatever is already there. `vibe new`
        // is for a project that does not exist; adopting an existing directory
        // is what `vibe scan` is for.
        if project_dir.exists() {
            return Err(CoreError::TargetExists { path: project_dir });
        }

        let manifest_file = manifest_path(&project_dir);
        let mut doc = ManifestDocument::scaffold(&manifest_file, name, &self.config.today_utc());
        if let Some(desc) = &req.description {
            doc.apply(crate::manifest::FieldEdit::SetDescription(Some(
                desc.clone(),
            )))?;
        }
        if req.status != Status::Active {
            doc.apply(crate::manifest::FieldEdit::SetProjectStatus(
                req.status.clone(),
            ))?;
        }

        let ops = vec![
            FileOp::CreateDir {
                path: project_dir.clone(),
            },
            FileOp::CreateDir {
                path: project_dir.join(".vibe"),
            },
            doc.into_create_op(),
        ];

        Ok(WritePlan::new(PlanIntent::New, project_dir, ops))
    }

    /// Execute a plan. The only method in this crate that writes to disk.
    pub fn apply(&self, plan: &WritePlan, rep: &dyn Reporter) -> Result<ApplyReport, CoreError> {
        plan::apply(plan, rep)
    }
}

/// Make a path absolute without requiring it to exist.
///
/// `canonicalize` is deliberately **not** used here, for two reasons: it fails
/// on a path that does not exist yet, which is the whole point of `vibe new`;
/// and on Windows it returns an extended-length `\\?\C:\...` form that then
/// leaks into every error message and every line of `--dry-run` output.
///
/// Lexical normalisation gives an absolute, `..`-free path that a human can
/// read. Containment still canonicalises both sides when it compares them
/// (`plan::validate_path`), so nothing is weakened by keeping the *displayed*
/// path in its ordinary form.
pub(crate) fn absolutize(p: &Path) -> Result<PathBuf, CoreError> {
    let absolute = if p.is_absolute() {
        p.to_path_buf()
    } else {
        let cwd = std::env::current_dir().map_err(|source| CoreError::Io {
            path: p.to_path_buf(),
            source,
        })?;
        cwd.join(p)
    };
    Ok(normalize_lexically(&absolute))
}

/// Resolve `.` and `..` textually, without touching the filesystem.
fn normalize_lexically(p: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}
