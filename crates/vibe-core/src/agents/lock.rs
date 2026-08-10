//! `.vibe/agents.lock` — the record of what **we** installed.
//!
//! This file is the entire ownership model. `.claude/agents/` will hold
//! hand-written agents and agents installed by other tools, and the rule is
//! that `vibe` never modifies or deletes a file it did not put there. The only
//! thing that distinguishes ours from theirs is an entry here.
//!
//! Which is why this is **not** a cache, despite resembling one. A corrupt
//! cache is discarded, because it can be rebuilt from disk. A corrupt lockfile
//! cannot be rebuilt: the hash of what we wrote is not recoverable by looking
//! at a file the user may since have edited. Discarding it would silently
//! convert every installed agent into an unowned one — a permanent loss of the
//! only ownership record. So it is reported, and ownership-dependent operations
//! refuse until the user resolves it (ADR-0006 §3a).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::CoreError;
use crate::manifest::SchemaVersion;

/// The lockfile format this build writes.
pub const LOCK_VERSION: SchemaVersion = SchemaVersion::new(1, 0);

/// The hash algorithm this build computes, as it appears in the file.
///
/// The prefix is the point. A bare hex string is a format with no migration
/// path: the day the algorithm changes, every existing entry becomes a hash of
/// unknown provenance that compares unequal, and every installed agent reports
/// as `Modified` — offering to overwrite work that was never touched. With a
/// prefix, an unrecognised algorithm is a *known unknown*: "cannot verify"
/// rather than "differs". That is the `NotAttempted`-versus-`Conflict`
/// distinction applied to hashing.
pub const HASH_PREFIX: &str = "b3:";

/// Hash agent file contents the way the lockfile records them.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> String {
    format!("{HASH_PREFIX}{}", blake3::hash(bytes).to_hex())
}

/// Whether a recorded hash was produced by an algorithm this build knows.
#[must_use]
pub fn hash_is_verifiable(recorded: &str) -> bool {
    recorded.starts_with(HASH_PREFIX)
}

/// One installed agent, as recorded by us.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct LockedAgent {
    pub name: String,
    /// Project-relative install location. Recorded rather than derived, so a
    /// future change to the layout does not orphan existing installs.
    pub path: String,
    /// What we wrote. The only way to tell "unmodified" from "edited".
    pub content_hash: String,
    /// The store commit this content came from.
    ///
    /// **Per agent, not per file.** A store-wide revision looks like the
    /// obvious normalisation and does not work: a crash partway through
    /// `update` would leave a partially-advanced global revision that describes
    /// every agent inaccurately, with no way to tell which were actually
    /// rewritten. Per-agent makes a half-finished update N independent states,
    /// each already named in the state table (ADR-0006 §4a).
    pub source_rev: String,
    /// Where it lived in the store. Distinguishes an upstream *rename* from a
    /// deletion.
    pub source_path: String,
    /// RFC 3339 UTC. Diagnostic only — never compared for staleness.
    pub installed_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Lockfile {
    pub version: SchemaVersion,
    /// Keyed by name, which keeps the file stable across runs and makes
    /// lookup by the identity every command uses O(log n).
    pub agents: BTreeMap<String, LockedAgent>,
}

/// The outcome of loading a lockfile.
///
/// **Not a `Result`**, and not for the same reason as [`crate::CacheLoad`].
/// The cache returns a non-`Result` because every failure is harmless; this
/// one does it because the failures are *not* harmless and each demands a
/// different, specific response — collapsing them into one error would lose
/// exactly the distinction that decides whether it is safe to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockLoad {
    Loaded(Lockfile),
    /// No lockfile. Nothing has been installed by us — which is exactly what a
    /// fresh clone looks like, so it is not an error.
    Absent,
    /// Present and unparseable. **Not discarded.** Ownership is unknowable
    /// until a human resolves it.
    Corrupt {
        why: String,
    },
    /// A major version this build cannot read. Ownership is unknowable.
    VersionUnsupported {
        found: SchemaVersion,
    },
    /// A higher minor. Readable; unknown fields must survive a write.
    LoadedNewerMinor {
        lock: Lockfile,
        found: SchemaVersion,
    },
}

impl LockLoad {
    /// The lockfile if one is usable, else an empty one.
    ///
    /// Note what this does **not** do: it does not treat `Corrupt` or
    /// `VersionUnsupported` as empty. Those return `None`, because "we own
    /// nothing" and "we cannot tell what we own" must not be the same answer —
    /// the first permits writes, the second must forbid them.
    #[must_use]
    pub fn usable(&self) -> Option<&Lockfile> {
        match self {
            LockLoad::Loaded(l) | LockLoad::LoadedNewerMinor { lock: l, .. } => Some(l),
            LockLoad::Absent => None,
            LockLoad::Corrupt { .. } | LockLoad::VersionUnsupported { .. } => None,
        }
    }

    /// Whether ownership is knowable at all.
    ///
    /// `false` means every ownership-dependent operation must refuse. It is
    /// deliberately distinct from "owns nothing".
    #[must_use]
    pub fn ownership_known(&self) -> bool {
        match self {
            LockLoad::Loaded(_) | LockLoad::LoadedNewerMinor { .. } | LockLoad::Absent => true,
            LockLoad::Corrupt { .. } | LockLoad::VersionUnsupported { .. } => false,
        }
    }

    /// Whether this build may write the lockfile back.
    #[must_use]
    pub fn writable(&self) -> bool {
        self.ownership_known()
    }

    #[must_use]
    pub fn note(&self) -> Option<String> {
        match self {
            LockLoad::Loaded(_) | LockLoad::Absent => None,
            LockLoad::LoadedNewerMinor { found, .. } => Some(format!(
                "agents.lock is version {found}, this build writes {LOCK_VERSION}; \
                 unrecognised fields will be preserved"
            )),
            LockLoad::Corrupt { why } => Some(format!(
                "agents.lock could not be read ({why}). \
                 vibe cannot tell which agents it installed, so it will not \
                 modify or remove any of them until this is resolved."
            )),
            LockLoad::VersionUnsupported { found } => Some(format!(
                "agents.lock is version {found}, this build reads {}.x. \
                 vibe cannot tell which agents it installed, so it will not \
                 modify or remove any of them. Upgrade vibe.",
                LOCK_VERSION.major
            )),
        }
    }
}

/// Where the lockfile lives for a project.
#[must_use]
pub fn lock_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".vibe").join("agents.lock")
}

/// Read and classify a lockfile.
pub fn load(project_dir: &Path) -> LockLoad {
    let path = lock_path(project_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LockLoad::Absent,
        Err(e) => return LockLoad::Corrupt { why: e.to_string() },
    };
    parse(&text)
}

pub(crate) fn parse(text: &str) -> LockLoad {
    let doc: toml_edit::DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            return LockLoad::Corrupt { why: e.to_string() };
        }
    };

    let version = match doc.get("lock_version").and_then(|v| v.as_str()) {
        Some(s) => match s.parse::<SchemaVersion>() {
            Ok(v) => v,
            Err(_) => {
                return LockLoad::Corrupt {
                    why: format!("unreadable lock_version `{s}`"),
                };
            }
        },
        None => {
            return LockLoad::Corrupt {
                why: "no lock_version".to_owned(),
            };
        }
    };

    if version.major != LOCK_VERSION.major {
        return LockLoad::VersionUnsupported { found: version };
    }

    let mut agents = BTreeMap::new();
    if let Some(array) = doc.get("agent").and_then(|a| a.as_array_of_tables()) {
        for tbl in array {
            let field = |k: &str| tbl.get(k).and_then(|v| v.as_str()).map(str::to_owned);
            let (Some(name), Some(path), Some(content_hash), Some(source_rev)) = (
                field("name"),
                field("path"),
                field("content_hash"),
                field("source_rev"),
            ) else {
                // A partial entry is corruption, not an entry to skip. Skipping
                // would silently convert that agent to unowned, which is the
                // one thing discarding-on-corruption was rejected for.
                return LockLoad::Corrupt {
                    why: "an [[agent]] entry is missing required fields".to_owned(),
                };
            };
            agents.insert(
                name.clone(),
                LockedAgent {
                    name,
                    path,
                    content_hash,
                    source_rev,
                    source_path: field("source_path").unwrap_or_default(),
                    installed_at: field("installed_at").unwrap_or_default(),
                },
            );
        }
    }

    let lock = Lockfile { version, agents };
    if version.minor > LOCK_VERSION.minor {
        LockLoad::LoadedNewerMinor {
            lock,
            found: version,
        }
    } else {
        LockLoad::Loaded(lock)
    }
}

/// Render a lockfile. Deterministic: entries are emitted in name order.
#[must_use]
pub fn render(lock: &Lockfile) -> String {
    let mut out = String::from(
        "# Generated by vibe. Local filesystem state - add to .gitignore.\n\
         # This records which agents vibe installed, so it never touches one it did not.\n",
    );
    out.push_str(&format!("lock_version = \"{LOCK_VERSION}\"\n"));
    for agent in lock.agents.values() {
        out.push_str("\n[[agent]]\n");
        out.push_str(&format!("name = {}\n", quote(&agent.name)));
        out.push_str(&format!("path = {}\n", quote(&agent.path)));
        out.push_str(&format!("content_hash = {}\n", quote(&agent.content_hash)));
        out.push_str(&format!("source_rev = {}\n", quote(&agent.source_rev)));
        out.push_str(&format!("source_path = {}\n", quote(&agent.source_path)));
        out.push_str(&format!("installed_at = {}\n", quote(&agent.installed_at)));
    }
    out
}

/// TOML basic-string quoting, via `toml_edit` so escaping is not hand-rolled.
fn quote(s: &str) -> String {
    toml_edit::Value::from(s).to_string().trim().to_owned()
}

/// The op that writes a lockfile, for inclusion in a [`crate::WritePlan`].
///
/// A lockfile change is never applied directly — it goes through the same
/// plan/apply path as every other write, so `--dry-run` shows it and the
/// containment rules apply to it.
pub fn write_op(project_dir: &Path, lock: &Lockfile) -> Result<crate::FileOp, CoreError> {
    let path = lock_path(project_dir);
    let after = render(lock);
    match std::fs::read_to_string(&path) {
        Ok(before) => Ok(crate::FileOp::UpdateFile {
            path,
            before,
            after,
            reason: crate::manifest::EditReason::FieldsUpdated,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(crate::FileOp::CreateFile {
            path,
            contents: after,
        }),
        Err(source) => Err(CoreError::Io { path, source }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Lockfile {
        let mut agents = BTreeMap::new();
        agents.insert(
            "code-reviewer".to_owned(),
            LockedAgent {
                name: "code-reviewer".to_owned(),
                path: ".claude/agents/code-reviewer.md".to_owned(),
                content_hash: content_hash(b"hello"),
                source_rev: "9c1d4e2".to_owned(),
                source_path: "engineering/code-reviewer.md".to_owned(),
                installed_at: "2026-08-10T09:14:22Z".to_owned(),
            },
        );
        Lockfile {
            version: LOCK_VERSION,
            agents,
        }
    }

    #[test]
    fn render_and_parse_round_trip() {
        let text = render(&sample());
        let LockLoad::Loaded(back) = parse(&text) else {
            panic!("should load: {text}");
        };
        assert_eq!(back, sample());
    }

    #[test]
    fn hashes_carry_their_algorithm() {
        let h = content_hash(b"contents");
        assert!(h.starts_with("b3:"), "{h}");
        assert!(hash_is_verifiable(&h));

        // The whole point of the prefix: a hash from an algorithm this build
        // does not know is *unverifiable*, not *different*. Treating it as
        // different would report every agent as Modified after an algorithm
        // change and offer to overwrite untouched work.
        assert!(!hash_is_verifiable("sha256:abc123"));
        assert!(
            !hash_is_verifiable("abc123"),
            "a bare hex string is unverifiable"
        );
    }

    #[test]
    fn different_content_hashes_differently() {
        assert_ne!(content_hash(b"a"), content_hash(b"b"));
        assert_eq!(content_hash(b"a"), content_hash(b"a"));
    }

    #[test]
    fn an_absent_lockfile_is_not_an_error_and_owns_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let load = load(dir.path());
        assert_eq!(load, LockLoad::Absent);
        assert!(load.note().is_none(), "a fresh clone is not an event");
        assert!(load.ownership_known(), "we know we own nothing");
        assert!(load.writable());
        assert!(load.usable().is_none());
    }

    /// The asymmetry with the cache, asserted rather than merely documented.
    #[test]
    fn a_corrupt_lockfile_is_not_treated_as_empty() {
        let load = parse("this is not toml {{{");
        assert!(matches!(load, LockLoad::Corrupt { .. }));

        // The distinction that matters: "we own nothing" would permit writing,
        // and would silently convert every installed agent to unowned.
        assert!(
            !load.ownership_known(),
            "a corrupt lockfile means ownership is UNKNOWN, not empty"
        );
        assert!(!load.writable(), "and writing must be refused");
        assert!(load.usable().is_none());
        assert!(load.note().is_some_and(|n| n.contains("will not")));
    }

    #[test]
    fn a_partial_entry_is_corruption_not_a_skipped_agent() {
        let load = parse("lock_version = \"1.0\"\n\n[[agent]]\nname = \"x\"\n");
        assert!(
            matches!(load, LockLoad::Corrupt { .. }),
            "skipping would silently disown that agent, got {load:?}"
        );
    }

    #[test]
    fn a_newer_major_makes_ownership_unknowable() {
        let load = parse("lock_version = \"9.0\"\n");
        assert_eq!(
            load,
            LockLoad::VersionUnsupported {
                found: SchemaVersion::new(9, 0)
            }
        );
        assert!(!load.ownership_known());
        assert!(!load.writable());
        assert!(load.note().is_some_and(|n| n.contains("Upgrade vibe")));
    }

    #[test]
    fn a_newer_minor_is_readable_and_writable() {
        let mut text = render(&sample());
        text = text.replace("lock_version = \"1.0\"", "lock_version = \"1.7\"");
        let load = parse(&text);
        let LockLoad::LoadedNewerMinor { lock, found } = &load else {
            panic!("expected a newer minor, got {load:?}");
        };
        assert_eq!(*found, SchemaVersion::new(1, 7));
        assert_eq!(lock.agents.len(), 1);
        assert!(load.ownership_known());
        assert!(load.writable());
    }

    #[test]
    fn a_missing_version_is_corruption_not_a_default() {
        // Defaulting to 1.0 would be guessing about a file whose whole purpose
        // is to be authoritative about ownership.
        assert!(matches!(
            parse("[[agent]]\nname = \"x\"\n"),
            LockLoad::Corrupt { .. }
        ));
    }

    #[test]
    fn names_needing_escaping_survive_a_round_trip() {
        let mut lock = sample();
        let odd = r#"weird "quoted" \ name"#;
        lock.agents.insert(
            odd.to_owned(),
            LockedAgent {
                name: odd.to_owned(),
                path: ".claude/agents/odd.md".to_owned(),
                content_hash: content_hash(b"x"),
                source_rev: "abc".to_owned(),
                source_path: "odd.md".to_owned(),
                installed_at: "2026-08-10T00:00:00Z".to_owned(),
            },
        );
        let LockLoad::Loaded(back) = parse(&render(&lock)) else {
            panic!("quoting is hand-rolled somewhere");
        };
        assert_eq!(back.agents.get(odd).map(|a| a.name.as_str()), Some(odd));
    }

    #[test]
    fn rendering_is_deterministic() {
        assert_eq!(render(&sample()), render(&sample()));
    }
}
