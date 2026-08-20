//! Errors carry *data*, never a human-facing remediation sentence.
//!
//! `vibectl` owns "did you mean", "run `vibe scan` first", and colour. This
//! crate owns paths, spans, key names and exit statuses. `anyhow` appears only
//! in the CLI, so a `?` here cannot accidentally erase structure (ADR-0001 §4).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::manifest::SchemaVersion;

/// Every failure this crate can produce.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    #[error("no manifest at {}", .path.display())]
    ManifestNotFound { path: PathBuf },

    #[error("manifest at {} is not valid TOML", .path.display())]
    ManifestSyntax {
        path: PathBuf,
        #[source]
        source: Box<toml_edit::TomlError>,
    },

    #[error(
        "manifest at {} uses schema {found}, this build reads {supported_major}.x",
        .path.display()
    )]
    SchemaMajorMismatch {
        path: PathBuf,
        found: SchemaVersion,
        supported_major: u16,
    },

    #[error("manifest at {} has an unreadable schema_version `{found}`", .path.display())]
    SchemaVersionUnreadable { path: PathBuf, found: String },

    #[error("manifest at {} is missing required field `{field}`", .path.display())]
    ManifestMissingField { path: PathBuf, field: String },

    #[error(
        "manifest at {}: field `{field}` should be {expected}, found {found}",
        .path.display()
    )]
    ManifestFieldType {
        path: PathBuf,
        field: String,
        expected: &'static str,
        found: &'static str,
    },

    #[error("{} already exists", .path.display())]
    TargetExists { path: PathBuf },

    #[error("{} changed on disk between plan and apply", .path.display())]
    PlanStale { path: PathBuf },

    #[error("{} is outside the plan's root {}", .path.display(), .root.display())]
    PathEscapesRoot { path: PathBuf, root: PathBuf },

    /// Rule 6 of ADR-0005 §10. `.git/hooks/*` executes with no configuration
    /// and no argument, so a write landing there converts an additive file
    /// write into code execution. `vibe` has no legitimate reason to write
    /// inside `.git/`, ever.
    #[error("{} is inside a .git directory, which vibe never writes to", .path.display())]
    PathInsideGitDir { path: PathBuf },

    #[error("io error at {}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A configured upstream URL that must not reach `git`'s argument vector.
    ///
    /// `why` is a fixed string rather than free prose because it is the one
    /// piece a user needs to act on, and the set of reasons is closed. See
    /// [`crate::agents::GitUrl`].
    #[error("refusing to use `{url}` as a store URL: {why}")]
    GitUrlRejected { url: String, why: &'static str },

    /// The configured store directory is a git repository pointing somewhere
    /// else.
    ///
    /// `update` does a `reset --hard`, so this is the check that stops a
    /// mistyped store path from throwing away the user's real work. Refusing is
    /// the only safe answer: we cannot tell a typo from a deliberate re-point,
    /// and one of those two readings is destructive.
    #[error(
        "{} is a git repository for `{}`, not the configured store `{expected}`",
        .path.display(),
        .found.as_deref().unwrap_or("(no origin remote)")
    )]
    StoreNotOurs {
        path: PathBuf,
        found: Option<String>,
        expected: String,
    },

    /// The store directory exists and is not a git repository at all.
    #[error("{} exists but is not a git repository", .path.display())]
    StoreNotARepository { path: PathBuf },

    /// A subprocess failed. `status` travels separately from the prose because
    /// the prose is whatever the tool wrote to stderr (ADR-0005 §6).
    // Both halves of the pair, not just the subcommand: `argv[0]` was once a
    // hard-coded "git", which rendered a failed `gh repo create` as
    // "git repo failed" — a message naming the wrong program entirely.
    #[error(
        "{} {} failed",
        .argv.first().map_or("", String::as_str),
        .argv.get(1).map_or("", String::as_str)
    )]
    ToolFailed {
        argv: Vec<String>,
        status: Option<i32>,
        stderr: String,
    },

    /// `git` could not be run. A fact about this machine, not about the store —
    /// the same distinction `SyncNotes::not_attempted` draws.
    #[error("git could not be run: {why}")]
    GitUnavailable { why: String },

    /// An ownership-dependent operation was asked for while the lockfile was
    /// unreadable. Carries the note so a caller need not re-derive it.
    #[error("{why}")]
    OwnershipUnknown { why: String },

    /// A template failed to render. A bug in a compiled-in template, not
    /// something a user's manifest should be able to cause - which is why the
    /// sparse-manifest tests in `render::engine` exist.
    #[error("could not render {target}: {why}")]
    RenderFailed { target: &'static str, why: String },

    /// `render` declined to write over what is already there.
    ///
    /// Carries the state rather than prose, so a caller can tell
    /// "you edited this, pass --force" from "this is not ours and --force will
    /// not help" - which are the two halves of ADR-0007 §4 and must not read
    /// the same.
    #[error("{} is {}", .path.display(), .state.as_str())]
    RenderRefused {
        path: PathBuf,
        state: crate::render::RenderState,
    },

    /// `vibe monitor install` declined to edit a settings file.
    ///
    /// **Reported, never repaired** (ADR-0011 §7). Every
    /// [`SettingsRefusal`](crate::monitor::SettingsRefusal) variant is a fact
    /// about a file vibe does not own, and carrying the refusal rather than
    /// prose is what lets a caller tell "your JSON is malformed" from "your
    /// config declares vibe's identity somewhere vibe cannot own" - which need
    /// different things from the user and must not read the same.
    #[error("{}: {}", .path.display(), .refusal.key())]
    SettingsRefused {
        path: PathBuf,
        refusal: crate::monitor::SettingsRefusal,
    },
}

impl CoreError {
    /// A stable identifier, safe to branch on. Unlike the `Display` text, this
    /// does not change when wording improves.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::ManifestNotFound { .. } => "VIBE_E_MANIFEST_NOT_FOUND",
            CoreError::ManifestSyntax { .. } => "VIBE_E_MANIFEST_SYNTAX",
            CoreError::SchemaMajorMismatch { .. } => "VIBE_E_SCHEMA_MAJOR_MISMATCH",
            CoreError::SchemaVersionUnreadable { .. } => "VIBE_E_SCHEMA_UNREADABLE",
            CoreError::ManifestMissingField { .. } => "VIBE_E_MANIFEST_MISSING_FIELD",
            CoreError::ManifestFieldType { .. } => "VIBE_E_MANIFEST_FIELD_TYPE",
            CoreError::TargetExists { .. } => "VIBE_E_TARGET_EXISTS",
            CoreError::PlanStale { .. } => "VIBE_E_PLAN_STALE",
            CoreError::PathEscapesRoot { .. } => "VIBE_E_PATH_ESCAPES_ROOT",
            CoreError::PathInsideGitDir { .. } => "VIBE_E_PATH_INSIDE_GIT_DIR",
            CoreError::Io { .. } => "VIBE_E_IO",
            CoreError::GitUrlRejected { .. } => "VIBE_E_GIT_URL_REJECTED",
            CoreError::StoreNotOurs { .. } => "VIBE_E_STORE_NOT_OURS",
            CoreError::StoreNotARepository { .. } => "VIBE_E_STORE_NOT_A_REPOSITORY",
            CoreError::ToolFailed { .. } => "VIBE_E_TOOL_FAILED",
            CoreError::GitUnavailable { .. } => "VIBE_E_GIT_UNAVAILABLE",
            CoreError::OwnershipUnknown { .. } => "VIBE_E_OWNERSHIP_UNKNOWN",
            CoreError::RenderFailed { .. } => "VIBE_E_RENDER_FAILED",
            CoreError::RenderRefused { .. } => "VIBE_E_RENDER_REFUSED",
            CoreError::SettingsRefused { .. } => "VIBE_E_SETTINGS_REFUSED",
        }
    }

    /// The path this error is about, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            CoreError::ManifestNotFound { path }
            | CoreError::ManifestSyntax { path, .. }
            | CoreError::SchemaMajorMismatch { path, .. }
            | CoreError::SchemaVersionUnreadable { path, .. }
            | CoreError::ManifestMissingField { path, .. }
            | CoreError::ManifestFieldType { path, .. }
            | CoreError::TargetExists { path }
            | CoreError::PlanStale { path }
            | CoreError::PathEscapesRoot { path, .. }
            | CoreError::PathInsideGitDir { path }
            | CoreError::StoreNotOurs { path, .. }
            | CoreError::StoreNotARepository { path }
            | CoreError::RenderRefused { path, .. }
            | CoreError::SettingsRefused { path, .. }
            | CoreError::Io { path, .. } => Some(path),
            CoreError::GitUrlRejected { .. }
            | CoreError::ToolFailed { .. }
            | CoreError::GitUnavailable { .. }
            | CoreError::RenderFailed { .. }
            | CoreError::OwnershipUnknown { .. } => None,
        }
    }

    /// The serializable projection. [`CoreError`] itself is deliberately not
    /// `Serialize`: its `#[source]` chain holds `std::io::Error` and
    /// `toml_edit::TomlError`, neither of which belongs on a wire (ADR-0001 §4).
    #[must_use]
    pub fn to_wire(&self) -> ErrorPayload {
        let mut params = BTreeMap::new();
        match self {
            CoreError::SchemaMajorMismatch {
                found,
                supported_major,
                ..
            } => {
                params.insert("found".to_owned(), found.to_string());
                params.insert("found_major".to_owned(), found.major.to_string());
                params.insert("found_minor".to_owned(), found.minor.to_string());
                params.insert("supported_major".to_owned(), supported_major.to_string());
            }
            CoreError::SchemaVersionUnreadable { found, .. } => {
                params.insert("found".to_owned(), found.clone());
            }
            CoreError::ManifestMissingField { field, .. } => {
                params.insert("field".to_owned(), field.clone());
            }
            CoreError::ManifestFieldType {
                field,
                expected,
                found,
                ..
            } => {
                params.insert("field".to_owned(), field.clone());
                params.insert("expected".to_owned(), (*expected).to_owned());
                params.insert("found".to_owned(), (*found).to_owned());
            }
            CoreError::PathEscapesRoot { root, .. } => {
                params.insert("root".to_owned(), root.display().to_string());
            }
            // `io::ErrorKind` travels as a stable discriminant because the
            // prose in the chain is OS-locale-dependent on Windows —
            // FormatMessage returns localised strings, so a consumer that keys
            // off the message is broken on an Italian machine (ADR-0005 §6).
            CoreError::Io { source, .. } => {
                params.insert("io_kind".to_owned(), format!("{:?}", source.kind()));
            }
            CoreError::GitUrlRejected { url, why } => {
                params.insert("url".to_owned(), url.clone());
                params.insert("why".to_owned(), (*why).to_owned());
            }
            CoreError::StoreNotOurs {
                found, expected, ..
            } => {
                params.insert("expected".to_owned(), expected.clone());
                if let Some(f) = found {
                    params.insert("found".to_owned(), f.clone());
                }
            }
            // `status` is the structured discriminant a consumer branches on;
            // `stderr` is git's prose and is carried alongside, never instead.
            // The refusal's stable key is the discriminant a consumer branches
            // on, for the same reason `ToolFailed` carries `status`: the
            // `Display` prose is a fallback, and a caller that keys off it is
            // reading a sentence rather than a fact.
            CoreError::SettingsRefused { refusal, .. } => {
                params.insert("refusal".to_owned(), refusal.key().to_owned());
            }
            CoreError::ToolFailed { status, argv, .. } => {
                if let Some(code) = status {
                    params.insert("status".to_owned(), code.to_string());
                }
                params.insert("argv".to_owned(), argv.join(" "));
            }
            _ => {}
        }

        let (path, path_lossy) = match self.path() {
            Some(p) => {
                let (s, lossy) = display_path(p);
                (Some(s), lossy)
            }
            None => (None, false),
        };

        ErrorPayload {
            code: self.code(),
            message: self.to_string(),
            path,
            path_lossy,
            params,
            chain: error_chain(self),
        }
    }
}

/// The wire shape of a [`CoreError`]: a stable `code`, named `params` for
/// interpolation, and prose carried *alongside* rather than instead.
///
/// A consumer renders its own sentence from `code` + `params`; the `message` is
/// a fallback, not a contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ErrorPayload {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// True when `path` lost bytes converting to UTF-8. Skipped when false, so
    /// it is invisible to `jq` in the overwhelmingly common case.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub path_lossy: bool,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub chain: Vec<String>,
}

/// Render a path for display, reporting whether the conversion was lossy.
///
/// Lossiness is confined to *labels*. Nothing addresses a project by this
/// string — that is what `ProjectId` (derived from raw path bytes) is for, so
/// two paths differing only in non-UTF-8 bytes can never collapse into
/// operating on the wrong project. See ADR-0005 §4.
pub(crate) fn display_path(p: &Path) -> (String, bool) {
    match p.to_str() {
        Some(s) => (s.to_owned(), false),
        None => (p.to_string_lossy().into_owned(), true),
    }
}

fn error_chain(err: &dyn std::error::Error) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = err.source();
    while let Some(e) = cur {
        out.push(e.to_string());
        cur = e.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_mismatch_carries_both_numbers_as_params() {
        let err = CoreError::SchemaMajorMismatch {
            path: PathBuf::from("/p/.vibe/project.toml"),
            found: SchemaVersion::new(3, 0),
            supported_major: 2,
        };
        let wire = err.to_wire();

        assert_eq!(wire.code, "VIBE_E_SCHEMA_MAJOR_MISMATCH");
        assert_eq!(wire.params["found"], "3.0");
        assert_eq!(wire.params["supported_major"], "2");
        // The prose names both numbers and the action, per ADR-0002 §3.
        assert!(
            wire.message.contains("schema 3.0") && wire.message.contains("2.x"),
            "message should name both versions, got: {}",
            wire.message
        );
    }

    #[test]
    fn io_errors_carry_a_stable_kind_not_just_localised_prose() {
        let err = CoreError::Io {
            path: PathBuf::from("/nope"),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        };
        let wire = err.to_wire();
        assert_eq!(wire.params["io_kind"], "PermissionDenied");
        assert_eq!(wire.chain, vec!["denied".to_owned()]);
    }

    #[test]
    fn every_variant_has_a_distinct_code() {
        let all = [
            CoreError::ManifestNotFound { path: "a".into() },
            CoreError::SchemaVersionUnreadable {
                path: "a".into(),
                found: "x".into(),
            },
            CoreError::ManifestMissingField {
                path: "a".into(),
                field: "f".into(),
            },
            CoreError::TargetExists { path: "a".into() },
            CoreError::PlanStale { path: "a".into() },
            CoreError::PathEscapesRoot {
                path: "a".into(),
                root: "b".into(),
            },
            CoreError::PathInsideGitDir { path: "a".into() },
        ];
        let mut codes: Vec<_> = all.iter().map(CoreError::code).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(before, codes.len(), "duplicate error codes: {codes:?}");
    }
}
