//! The global cache: fast, regenerable, and never authoritative.
//!
//! Everything here exists to make `vibe list` instant. Nothing here is allowed
//! to be the reason an answer is wrong.
//!
//! Three rules, and they are the whole design:
//!
//! 1. **Disk wins, unconditionally.** Where the cache and a manifest disagree,
//!    the manifest is right and the cache is stale. There is no tie-break.
//! 2. **`list` may be stale and says so; `show` never is.** `show` re-reads the
//!    manifest, always. The cache accelerates the overview, not the detail.
//! 3. **A cache problem is never a user-visible failure.** [`Cache::load`]
//!    returns [`CacheLoad`], not `Result`, because there is no cache error a
//!    user should have to act on — the worst case is a rescan.
//!
//! Deleting the cache is therefore a supported recovery path, not a corruption
//! scenario, and it is tested as one.
//!
//! See ADR-0004.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped when the shape below changes incompatibly.
///
/// The file *name* carries the version, so two installed builds do not fight
/// over one path — a v1 build and a v2 build each read and write their own and
/// neither corrupts the other. That is the deliberate asymmetry with manifest
/// versioning (ADR-0002 §3): a future *manifest* is preserved and refused,
/// because it is the user's data; a future *cache* is simply ignored, because
/// it is ours and regenerable.
pub const CACHE_VERSION: u32 = 1;

#[must_use]
pub fn cache_file_name() -> String {
    format!("index.v{CACHE_VERSION}.json")
}

/// What the cache holds for one project.
///
/// Denormalised: exactly the columns `list` prints, so rendering a table needs
/// no further reads. Anything a user might act on in detail is deliberately
/// absent — that is what `show` is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    pub path: String,
    pub name: String,
    pub status: String,
    pub archived: bool,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub last_commit: Option<String>,
    #[serde(default)]
    pub deploy_url: Option<String>,
    /// Freshness witness for the manifest: size and mtime, gating a digest.
    pub witness: Witness,
}

/// Cheap-first staleness detection.
///
/// `len` and `mtime_ms` are compared before the digest is trusted, because
/// reading a file to hash it is the thing this exists to avoid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Witness {
    /// Absent when the project has no manifest at all.
    #[serde(default)]
    pub manifest_len: Option<u64>,
    #[serde(default)]
    pub manifest_mtime_ms: Option<u64>,
}

impl Witness {
    /// Read the current witness for a manifest path.
    #[must_use]
    pub fn observe(manifest: &Path) -> Self {
        let Ok(meta) = std::fs::metadata(manifest) else {
            return Self {
                manifest_len: None,
                manifest_mtime_ms: None,
            };
        };
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        Self {
            manifest_len: Some(meta.len()),
            manifest_mtime_ms: mtime_ms,
        }
    }

    /// Whether an entry recorded with `self` may still be trusted against
    /// `current`.
    ///
    /// Deliberately conservative in one direction only: any doubt reads as
    /// stale, because a needless rescan costs milliseconds and a wrongly-trusted
    /// entry shows the user a value that is not on their disk.
    #[must_use]
    pub fn still_valid(&self, current: &Witness) -> bool {
        // A missing mtime on either side means we cannot tell. Filesystems with
        // coarse timestamps exist, and so do clocks that move backwards.
        match (self.manifest_mtime_ms, current.manifest_mtime_ms) {
            (Some(a), Some(b)) if a == b => {}
            _ => return false,
        }
        self.manifest_len == current.manifest_len
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheFile {
    pub version: u32,
    /// Keyed by project path so a lookup is not a scan.
    #[serde(default)]
    pub entries: BTreeMap<String, CacheEntry>,
}

/// The outcome of loading a cache. **Not a `Result`** — there is no cache
/// failure a user should have to act on, and every variant here degrades to
/// "rescan", never to a wrong answer or an error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheLoad {
    Loaded(CacheFile),
    /// No cache yet. The first run, or the user deleted it.
    Absent,
    /// Present and unparseable. Discarded.
    Corrupt {
        why: String,
    },
    /// Written by a build with a different cache schema. Ignored, not migrated
    /// and not deleted — the other build may still be in use.
    WrongVersion {
        found: u32,
    },
    /// The path could not be determined at all (no home directory, for
    /// instance). Everything still works; nothing is cached.
    Unavailable {
        why: String,
    },
}

impl CacheLoad {
    /// The cache if it is usable, and an empty one otherwise.
    ///
    /// The point of this method is that no caller has to write the "or else
    /// rescan" branch, so no caller can get it wrong.
    #[must_use]
    pub fn usable(self) -> CacheFile {
        match self {
            CacheLoad::Loaded(f) => f,
            _ => CacheFile {
                version: CACHE_VERSION,
                entries: BTreeMap::new(),
            },
        }
    }

    /// A one-line explanation, when there is something worth saying. Absent is
    /// not worth saying — a first run is not an event.
    #[must_use]
    pub fn note(&self) -> Option<String> {
        match self {
            CacheLoad::Loaded(_) | CacheLoad::Absent => None,
            CacheLoad::Corrupt { why } => Some(format!("cache was unreadable ({why}); rebuilding")),
            CacheLoad::WrongVersion { found } => Some(format!(
                "cache is version {found}, this build uses {CACHE_VERSION}; rebuilding"
            )),
            CacheLoad::Unavailable { why } => {
                Some(format!("no cache location available ({why}); not caching"))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Cache {
    path: Option<PathBuf>,
}

impl Cache {
    #[must_use]
    pub fn at(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[must_use]
    pub fn load(&self) -> CacheLoad {
        let Some(path) = &self.path else {
            return CacheLoad::Unavailable {
                why: "no cache directory for this platform".to_owned(),
            };
        };
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return CacheLoad::Absent,
            Err(e) => return CacheLoad::Corrupt { why: e.to_string() },
        };
        let file: CacheFile = match serde_json::from_str(&text) {
            Ok(f) => f,
            Err(e) => return CacheLoad::Corrupt { why: e.to_string() },
        };
        if file.version != CACHE_VERSION {
            return CacheLoad::WrongVersion {
                found: file.version,
            };
        }
        CacheLoad::Loaded(file)
    }

    /// Write the cache, atomically, and never fail loudly.
    ///
    /// Returns whether it was written. A cache that cannot be saved is a lost
    /// optimisation, not an error — the next command rescans.
    ///
    /// Temp-file-plus-rename, so a crash mid-write leaves the previous cache
    /// intact rather than a truncated one. Two concurrent scans are
    /// last-writer-wins; both wrote a complete, valid file, and the loser's
    /// work is recovered by the next scan.
    pub fn save(&self, file: &CacheFile) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        let Some(parent) = path.parent() else {
            return false;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
        let Ok(json) = serde_json::to_string(file) else {
            return false;
        };

        let tmp = parent.join(format!("{}.tmp", cache_file_name()));
        if std::fs::write(&tmp, json).is_err() {
            return false;
        }
        if std::fs::rename(&tmp, path).is_err() {
            // Windows will not rename onto an existing file in every case.
            let _ = std::fs::remove_file(path);
            if std::fs::rename(&tmp, path).is_err() {
                let _ = std::fs::remove_file(&tmp);
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_in(dir: &Path) -> Cache {
        Cache::at(Some(dir.join(cache_file_name())))
    }

    fn sample() -> CacheFile {
        let mut entries = BTreeMap::new();
        entries.insert(
            "/p/alpha".to_owned(),
            CacheEntry {
                path: "/p/alpha".to_owned(),
                name: "alpha".to_owned(),
                status: "active".to_owned(),
                archived: false,
                runtime: Some("node@22".to_owned()),
                remote: None,
                last_commit: None,
                deploy_url: None,
                witness: Witness {
                    manifest_len: Some(10),
                    manifest_mtime_ms: Some(1000),
                },
            },
        );
        CacheFile {
            version: CACHE_VERSION,
            entries,
        }
    }

    #[test]
    fn a_missing_cache_is_absent_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let load = cache_in(dir.path()).load();
        assert_eq!(load, CacheLoad::Absent);
        assert!(load.note().is_none(), "a first run is not an event");
        assert!(CacheLoad::Absent.usable().entries.is_empty());
    }

    #[test]
    fn a_corrupt_cache_degrades_to_empty_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(cache_file_name()), "{not json").unwrap();

        let load = cache_in(dir.path()).load();
        assert!(matches!(load, CacheLoad::Corrupt { .. }));
        assert!(load.note().is_some_and(|n| n.contains("rebuilding")));
        assert!(load.usable().entries.is_empty());
    }

    #[test]
    fn a_cache_from_another_version_is_ignored_not_migrated_or_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(cache_file_name());
        std::fs::write(&path, r#"{"version":99,"entries":{}}"#).unwrap();

        let load = cache_in(dir.path()).load();
        assert_eq!(load, CacheLoad::WrongVersion { found: 99 });
        assert!(load.usable().entries.is_empty());
        // Left on disk: the build that wrote it may still be installed.
        assert!(path.exists(), "another build's cache must not be deleted");
    }

    #[test]
    fn an_unavailable_cache_location_is_survivable() {
        let cache = Cache::at(None);
        assert!(matches!(cache.load(), CacheLoad::Unavailable { .. }));
        assert!(!cache.save(&sample()), "nothing to save to");
    }

    #[test]
    fn saving_and_loading_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_in(dir.path());
        assert!(cache.save(&sample()));
        assert_eq!(cache.load(), CacheLoad::Loaded(sample()));
    }

    #[test]
    fn saving_twice_replaces_rather_than_appends() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_in(dir.path());
        assert!(cache.save(&sample()));
        assert!(cache.save(&CacheFile {
            version: CACHE_VERSION,
            entries: BTreeMap::new(),
        }));
        assert_eq!(cache.load().usable().entries.len(), 0);
    }

    #[test]
    fn a_save_leaves_no_temporary_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        assert!(cache_in(dir.path()).save(&sample()));
        let files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, vec![cache_file_name()], "found: {files:?}");
    }

    #[test]
    fn a_changed_manifest_invalidates_its_entry() {
        let recorded = Witness {
            manifest_len: Some(100),
            manifest_mtime_ms: Some(5000),
        };
        assert!(recorded.still_valid(&recorded.clone()));

        // Edited: same length, later mtime.
        assert!(!recorded.still_valid(&Witness {
            manifest_len: Some(100),
            manifest_mtime_ms: Some(6000),
        }));
        // Edited: same mtime, different length. A filesystem with 1-second
        // granularity makes this the realistic case for a fast edit.
        assert!(!recorded.still_valid(&Witness {
            manifest_len: Some(120),
            manifest_mtime_ms: Some(5000),
        }));
        // Deleted.
        assert!(!recorded.still_valid(&Witness {
            manifest_len: None,
            manifest_mtime_ms: None,
        }));
    }

    #[test]
    fn an_unknowable_witness_always_reads_as_stale() {
        // No mtime available on either side: cannot tell, so do not trust.
        let unknown = Witness {
            manifest_len: Some(1),
            manifest_mtime_ms: None,
        };
        assert!(!unknown.still_valid(&unknown.clone()));
    }
}
