//! The only I/O surface a detector gets.
//!
//! Reads are memoized per project, so `package.json` is read once even though
//! three detectors want it, and size-capped so a pathological 400 MB manifest
//! cannot consume the scan budget.
//!
//! Some files are **never** readable, at the API level: a detector cannot opt
//! in, because the refusal lives in [`ReadCache`] rather than in a rule each
//! detector is asked to remember. The registry indexes secrets' *names* — from
//! `.env.example` — and never their values, so a scan cannot leak a credential
//! into a manifest that then gets committed (ADR-0003 §2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use crate::exec::{CommandOutput, ProcessRunner};
use crate::walk::FileIndex;

/// A detector could not produce findings.
///
/// Distinct from "found nothing": the user can act on `Unreadable`, and the
/// scan report says so rather than presenting an empty field as a clean bill
/// of health.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DetectError {
    Unreadable { path: String, why: String },
    Timeout,
    NotAttempted { why: String },
}

impl std::fmt::Display for DetectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DetectError::Unreadable { path, why } => write!(f, "could not read {path}: {why}"),
            DetectError::Timeout => f.write_str("timed out"),
            DetectError::NotAttempted { why } => write!(f, "not attempted: {why}"),
        }
    }
}

/// A file whose *contents* this tool must never read.
///
/// `.env.example` and `.env.sample` are explicitly allowed and are the only
/// source of `deploy.env_required` — key names, never values.
#[must_use]
pub fn is_forbidden_to_read(rel: &str) -> bool {
    let base = rel.rsplit('/').next().unwrap_or(rel);

    // Checked first: `.env.example` would otherwise be caught by the `.env.`
    // prefix rule below, and it is the one dotenv file we depend on.
    if base == ".env.example" || base == ".env.sample" || base == ".env.template" {
        return false;
    }

    base == ".env"
        || base.starts_with(".env.")
        || base.ends_with(".pem")
        || base.ends_with(".key")
        || base.ends_with(".p12")
        || base.ends_with(".pfx")
        || base.starts_with("id_rsa")
        || base.starts_with("id_ed25519")
        || base.starts_with("id_ecdsa")
        || base == ".npmrc"
        || base == ".netrc"
        || base == "_netrc"
        || base == ".pgpass"
        || base == "credentials"
        || base == ".git-credentials"
        || base == "secrets.yml"
        || base == "secrets.yaml"
}

/// A file larger than this is not worth reading to find a version string.
const MAX_READ_BYTES: u64 = 1024 * 1024;

/// Memoized, size-capped, secret-refusing reads rooted at one project.
#[derive(Debug)]
pub struct ReadCache {
    root: PathBuf,
    cache: Mutex<BTreeMap<String, Result<Arc<str>, DetectError>>>,
}

impl ReadCache {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn read_text(&self, rel: &str) -> Result<Arc<str>, DetectError> {
        if is_forbidden_to_read(rel) {
            return Err(DetectError::NotAttempted {
                why: format!("{rel} may contain secrets and is never read"),
            });
        }

        {
            let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(hit) = cache.get(rel) {
                return hit.clone();
            }
        }

        let result = self.read_uncached(rel);

        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.insert(rel.to_owned(), result.clone());
        result
    }

    fn read_uncached(&self, rel: &str) -> Result<Arc<str>, DetectError> {
        let path = self.root.join(rel);
        let unreadable = |why: String| DetectError::Unreadable {
            path: rel.to_owned(),
            why,
        };

        let meta = std::fs::metadata(&path).map_err(|e| unreadable(e.to_string()))?;
        if meta.len() > MAX_READ_BYTES {
            return Err(unreadable(format!(
                "{} bytes exceeds the {MAX_READ_BYTES} byte read cap",
                meta.len()
            )));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| unreadable(e.to_string()))?;
        Ok(Arc::from(text.as_str()))
    }
}

/// What a detector is handed. It has no other way to touch the world.
pub struct DetectCtx<'a> {
    pub root: &'a Path,
    pub files: &'a FileIndex,
    deadline: Option<Instant>,
    exec: &'a dyn ProcessRunner,
    reads: &'a ReadCache,
}

impl std::fmt::Debug for DetectCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetectCtx")
            .field("root", &self.root)
            .field("files", &self.files.len())
            .finish_non_exhaustive()
    }
}

impl<'a> DetectCtx<'a> {
    pub fn new(
        root: &'a Path,
        files: &'a FileIndex,
        reads: &'a ReadCache,
        exec: &'a dyn ProcessRunner,
        deadline: Option<Instant>,
    ) -> Self {
        Self {
            root,
            files,
            deadline,
            exec,
            reads,
        }
    }

    pub fn read_text(&self, rel: &str) -> Result<Arc<str>, DetectError> {
        self.reads.read_text(rel)
    }

    /// Run `git` in the project directory.
    ///
    /// The runner constructs the child environment rather than inheriting it,
    /// so `GIT_SSH_COMMAND` and friends from the caller's shell cannot reach
    /// a subprocess this tool spawns (ADR-0005 §10 rule 3).
    pub fn git(&self, args: &[&str]) -> Result<CommandOutput, DetectError> {
        self.exec.run_git(self.root, args)
    }

    /// Whether the time budget for this project is spent.
    ///
    /// A detector that finds this true returns nothing. Finishing on time with
    /// a field honestly marked unknown is the correct outcome; finishing on
    /// time with an invented value is the failure the whole model exists to
    /// prevent (ADR-0003 §8).
    #[must_use]
    pub fn expired(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_is_never_read_but_its_example_is() {
        assert!(is_forbidden_to_read(".env"));
        assert!(is_forbidden_to_read(".env.local"));
        assert!(is_forbidden_to_read(".env.production"));
        assert!(is_forbidden_to_read("sub/dir/.env"));

        // The one dotenv file the tool depends on, and the ordering that makes
        // it work: `.env.example` starts with `.env.` and must still be allowed.
        assert!(!is_forbidden_to_read(".env.example"));
        assert!(!is_forbidden_to_read(".env.sample"));
    }

    #[test]
    fn key_material_and_credential_files_are_never_read() {
        for f in [
            "server.pem",
            "private.key",
            "id_rsa",
            "id_ed25519",
            "cert.p12",
            ".npmrc",
            ".netrc",
            ".pgpass",
            "credentials",
            ".git-credentials",
            "secrets.yaml",
        ] {
            assert!(is_forbidden_to_read(f), "{f} should never be read");
        }
    }

    #[test]
    fn ordinary_manifests_are_readable() {
        for f in [
            "package.json",
            "Cargo.toml",
            "go.mod",
            "pyproject.toml",
            "composer.json",
            "vercel.json",
            "netlify.toml",
        ] {
            assert!(!is_forbidden_to_read(f), "{f} should be readable");
        }
    }

    #[test]
    fn the_read_cache_refuses_secrets_at_the_api_level() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "SUPABASE_KEY=hunter2").unwrap();

        let cache = ReadCache::new(dir.path());
        let err = cache.read_text(".env").expect_err("must refuse");
        assert!(matches!(err, DetectError::NotAttempted { .. }));

        // And the refusal is not merely advisory: the secret never enters the
        // process, so it cannot leak through an error message either.
        assert!(!format!("{err}").contains("hunter2"));
    }

    #[test]
    fn oversized_files_are_refused_rather_than_read() {
        let dir = tempfile::tempdir().unwrap();
        let big = "x".repeat((MAX_READ_BYTES + 1) as usize);
        std::fs::write(dir.path().join("package.json"), big).unwrap();

        let cache = ReadCache::new(dir.path());
        let err = cache.read_text("package.json").expect_err("must refuse");
        assert!(matches!(err, DetectError::Unreadable { .. }));
        assert!(format!("{err}").contains("read cap"));
    }

    #[test]
    fn reads_are_memoized() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one").unwrap();
        let cache = ReadCache::new(dir.path());

        let first = cache.read_text("a.txt").unwrap();
        std::fs::write(dir.path().join("a.txt"), "two").unwrap();
        let second = cache.read_text("a.txt").unwrap();

        assert_eq!(&*first, "one");
        assert_eq!(&*second, "one", "the second read came from the cache");
    }
}
