//! Finding projects, and indexing each one exactly once.
//!
//! Two different walks, deliberately:
//!
//! - [`discover_projects`] is shallow and structural. It answers "which of
//!   these directories is a project?" and stops descending the moment it knows,
//!   because what is *inside* a project is that project's business.
//! - [`FileIndex::build`] indexes one project. It runs once per project, and
//!   every detector is gated against its result rather than being allowed to
//!   touch the disk itself. That gating is the performance contract: on a Go
//!   repo, the Node, Python and PHP detectors never run and never stat
//!   anything (ADR-0003 §2).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Directories that are never worth descending into. Pruning these is most of
/// the reason a scan is fast — `node_modules` alone can hold more files than
/// the rest of a developer's home directory combined.
pub const PRUNE_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "dist",
    "build",
    "vendor",
    ".next",
    ".nuxt",
    ".svelte-kit",
    "venv",
    ".venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".tox",
    ".gradle",
    ".terraform",
    "Pods",
    ".svn",
    ".hg",
    ".idea",
    "coverage",
];

/// A file or directory whose presence means "this directory is a project".
const PROJECT_MARKERS: &[&str] = &[
    ".vibe",
    ".git",
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "requirements.txt",
    "go.mod",
    "composer.json",
];

/// Stop a pathological repository from consuming the whole scan budget.
const MAX_INDEXED_ENTRIES: usize = 20_000;

/// How deep below a scan root to look for project directories.
///
/// Three is enough for `~/projects/<name>` and `~/projects/<client>/<name>`
/// without turning a scan of a home directory into a full filesystem crawl.
pub const DEFAULT_DISCOVERY_DEPTH: usize = 3;

/// What one project directory contains, as far as detection is concerned.
#[derive(Debug, Clone, Default)]
pub struct FileIndex {
    /// Relative paths with forward slashes, e.g. `.vercel/project.json`.
    files: BTreeSet<String>,
    /// Base names of files **directly at the project root**.
    ///
    /// Root-level, not any-depth, and the distinction is load-bearing. Every
    /// stack detector reads a fixed root-relative path (`package.json`,
    /// `Cargo.toml`, …), so gating them on a name found *anywhere* fires a
    /// detector that then reads a file which does not exist. In a Go+Node
    /// monorepo with `web/package.json`, that produced
    /// `runtime = Unknown{Unreadable, path: "package.json"}` — a specific,
    /// actionable-looking failure about a file the tool invented. Reporting a
    /// fabricated read error is worse than reporting nothing.
    root_names: BTreeSet<String>,
    /// Extensions present, without the dot.
    extensions: BTreeSet<String>,
    /// Directory names directly at the project root, including pruned ones
    /// such as `.git`.
    root_dirs: BTreeSet<String>,
    /// True when the entry cap was hit, so callers know the index is partial
    /// rather than assuming absence means absence.
    truncated: bool,
}

impl FileIndex {
    /// Index one project directory.
    pub fn build(root: &Path) -> Self {
        let mut index = FileIndex::default();

        // Root entries are indexed first and unconditionally, before the
        // budgeted walk. Every detector reads a root-relative path, so if the
        // entry cap were reached while walking an `assets/` directory that
        // sorts before `go.mod`, the project's own manifest would be missing
        // from the index and the field would report `no_evidence` — the disk
        // was not silent, the walk simply never got there. A directory listing
        // of one directory is not worth budgeting.
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    index.root_dirs.insert(name);
                } else {
                    if let Some(ext) = std::path::Path::new(&name)
                        .extension()
                        .and_then(|e| e.to_str())
                    {
                        index.extensions.insert(ext.to_owned());
                    }
                    index.files.insert(name.clone());
                    index.root_names.insert(name);
                }
            }
        }

        let walker = ignore::WalkBuilder::new(root)
            // `.vibe`, `.git`, `.env.example` and `.vercel/` are all hidden, and
            // all load-bearing. The default of skipping hidden entries would
            // make the tool blind to its own manifest.
            .hidden(false)
            // `.gitignore` is deliberately NOT honoured. `vercel link` appends
            // `.vercel` to it and the Netlify CLI does the same with
            // `.netlify`, so respecting it made the deploy detectors blind on
            // exactly the projects that are actually deployed — and reported
            // that blindness as `no_evidence`, which claims the disk was
            // silent when it was not. The expensive directories are pruned by
            // name below regardless, and `MAX_INDEXED_ENTRIES` bounds the rest.
            .git_ignore(false)
            .git_global(false)
            .parents(false)
            .require_git(false)
            .max_depth(Some(6))
            .filter_entry(|entry| {
                let name = entry.file_name().to_string_lossy();
                !PRUNE_DIRS.contains(&name.as_ref())
            })
            .build();

        for entry in walker.flatten() {
            if index.files.len() >= MAX_INDEXED_ENTRIES {
                index.truncated = true;
                break;
            }
            let Ok(rel) = entry.path().strip_prefix(root) else {
                continue;
            };
            if rel.as_os_str().is_empty() {
                continue;
            }
            let rel_str = rel.to_string_lossy().replace('\\', "/");

            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            let at_root = !rel_str.contains('/');
            if at_root {
                if let Some(name) = entry.file_name().to_str() {
                    if is_dir {
                        index.root_dirs.insert(name.to_owned());
                    } else {
                        index.root_names.insert(name.to_owned());
                    }
                }
            }
            if !is_dir {
                if let Some(ext) = rel.extension().and_then(|e| e.to_str()) {
                    index.extensions.insert(ext.to_owned());
                }
                index.files.insert(rel_str);
            }
        }

        // Pruned directories still count as present. `.git` is the case that
        // matters: the git detector needs to know the repository exists, and
        // descending into it would be both pointless and slow.
        for pruned in PRUNE_DIRS {
            if root.join(pruned).is_dir() {
                index.root_dirs.insert((*pruned).to_owned());
            }
        }

        // In a git **worktree or submodule, `.git` is a file**, not a
        // directory — a one-line pointer at the real git directory. Testing
        // only `is_dir()` made the git detector never fire on those, so every
        // worktree silently reported no remote and no last commit as
        // `no_evidence`. It is recorded as a directory here because
        // `Interest::DirName(".git")` is the question the detector asks, and
        // the answer it wants is "is this a repository", not "what inode type
        // is this".
        if root.join(".git").exists() {
            index.root_dirs.insert(".git".to_owned());
        }

        index
    }

    /// A file with this name sits directly in the project root.
    #[must_use]
    pub fn has_file(&self, name: &str) -> bool {
        self.root_names.contains(name)
    }

    /// A directory with this name sits directly in the project root.
    #[must_use]
    pub fn has_dir(&self, name: &str) -> bool {
        self.root_dirs.contains(name)
    }

    #[must_use]
    pub fn has_extension(&self, ext: &str) -> bool {
        self.extensions.contains(ext)
    }

    /// Root-level file names starting with `prefix` and ending with `suffix`,
    /// in deterministic order. For `requirements*.txt`.
    ///
    /// Root-level for the same reason as [`Self::has_file`], and with a second
    /// consequence: `docs/requirements.txt` is a Sphinx build dependency, not a
    /// statement about the project's runtime. Matching it at any depth made
    /// every Go repository with Sphinx docs report a runtime conflict between
    /// `go` and `python`.
    #[must_use]
    pub fn root_files_matching(&self, prefix: &str, suffix: &str) -> Vec<&str> {
        self.root_names
            .iter()
            .filter(|base| base.starts_with(prefix) && base.ends_with(suffix))
            .map(String::as_str)
            .collect()
    }

    #[must_use]
    pub fn contains_path(&self, rel: &str) -> bool {
        self.files.contains(rel)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.root_dirs.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the entry cap was hit. A caller must not read absence as
    /// evidence when this is true.
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// Whether a directory is itself a project.
#[must_use]
pub fn is_project_dir(dir: &Path) -> bool {
    PROJECT_MARKERS.iter().any(|m| dir.join(m).exists())
}

/// What a discovery walk found, and what it declined to look at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovered {
    pub projects: Vec<PathBuf>,
    /// Directories not descended into because the depth limit was reached.
    ///
    /// Absence of a project below one of these is **not** evidence that there
    /// is no project there. Reporting "no projects found" after declining to
    /// look is the honest-detection rule failing at the traversal layer rather
    /// than the detector layer, and it is exactly as misleading — so the walk
    /// says where it stopped and the caller surfaces it.
    pub depth_limited: Vec<PathBuf>,
}

/// Find project directories at or below `root`.
///
/// Stops descending as soon as a directory is identified as a project, and
/// never descends into a pruned directory. Results are sorted, so two scans of
/// an unchanged tree produce the same order — a requirement for stable
/// snapshots and for `vibe sync` not to churn.
#[must_use]
pub fn discover_projects(root: &Path, max_depth: usize) -> Discovered {
    let mut found = Discovered::default();
    visit(root, 0, max_depth, &mut found);
    found.projects.sort();
    found.depth_limited.sort();
    found
}

fn visit(dir: &Path, depth: usize, max_depth: usize, out: &mut Discovered) {
    if is_project_dir(dir) {
        out.projects.push(dir.to_path_buf());
        return;
    }
    if depth >= max_depth {
        // Only record a stop if there was somewhere further to go. A leaf
        // directory at the depth limit hides nothing.
        if std::fs::read_dir(dir).is_ok_and(|mut e| {
            e.any(|entry| entry.is_ok_and(|entry| entry.file_type().is_ok_and(|t| t.is_dir())))
        }) {
            out.depth_limited.push(dir.to_path_buf());
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        // An unreadable directory is not an error worth failing a whole scan
        // for. It contributes nothing and the scan continues.
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if PRUNE_DIRS.contains(&name.as_ref()) {
            continue;
        }
        visit(&path, depth + 1, max_depth, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, contents: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, contents).unwrap();
    }

    #[test]
    fn indexes_hidden_files_because_the_tool_depends_on_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, ".vibe/project.toml", "x");
        write(root, ".env.example", "API_KEY=");
        write(root, ".vercel/project.json", "{}");

        let index = FileIndex::build(root);
        assert!(index.contains_path(".vibe/project.toml"));
        assert!(index.has_file(".env.example"));
        assert!(index.contains_path(".vercel/project.json"));
    }

    #[test]
    fn prunes_the_expensive_directories_but_still_records_dot_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "package.json", "{}");
        write(root, "node_modules/left-pad/index.js", "x");
        write(root, "target/debug/thing", "x");
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        write(root, ".git/config", "x");

        let index = FileIndex::build(root);
        assert!(index.has_file("package.json"));
        assert!(
            !index.contains_path("node_modules/left-pad/index.js"),
            "node_modules must not be descended into"
        );
        assert!(!index.contains_path("target/debug/thing"));
        assert!(
            !index.contains_path(".git/config"),
            ".git contents are not indexed"
        );
        assert!(
            index.has_dir(".git"),
            "but the repository's existence is recorded"
        );
    }

    #[test]
    fn discovery_stops_at_a_project_and_does_not_descend_into_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "alpha/package.json", "{}");
        write(root, "alpha/packages/inner/package.json", "{}");
        write(root, "beta/Cargo.toml", "");
        write(root, "notaproject/readme.md", "");

        let found = discover_projects(root, DEFAULT_DISCOVERY_DEPTH);
        assert_eq!(found.projects, vec![root.join("alpha"), root.join("beta")]);
    }

    #[test]
    fn discovery_is_sorted_so_two_scans_agree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["zeta", "alpha", "mu"] {
            write(root, &format!("{name}/go.mod"), "module x");
        }
        let a = discover_projects(root, DEFAULT_DISCOVERY_DEPTH);
        let b = discover_projects(root, DEFAULT_DISCOVERY_DEPTH);
        assert_eq!(a, b);
        assert_eq!(
            a.projects
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["alpha", "mu", "zeta"]
        );
    }

    #[test]
    fn a_depth_limited_walk_says_where_it_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a/b/c/d/package.json", "{}");

        // Depth 2 cannot reach the project at depth 4.
        let found = discover_projects(root, 2);
        assert!(found.projects.is_empty());
        assert!(
            !found.depth_limited.is_empty(),
            "declining to look must be reported, not silently returned as absence"
        );

        // Deep enough, and it is found with nothing withheld.
        let found = discover_projects(root, 4);
        assert_eq!(found.projects, vec![root.join("a/b/c/d")]);
        assert!(found.depth_limited.is_empty());
    }

    #[test]
    fn an_empty_directory_is_not_a_project() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_project_dir(dir.path()));
        assert!(discover_projects(dir.path(), 3).projects.is_empty());
    }

    #[test]
    fn a_directory_with_only_a_readme_is_not_a_project() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "README.md", "# notes");
        assert!(!is_project_dir(dir.path()));
    }
}
