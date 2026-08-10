//! Evidence: the citation that must exist before a value may.
//!
//! There is no `Evidence::none()`, no `Evidence::default()`, and no
//! `Evidence::inferred()`. Every constructor takes a source. That is the whole
//! trick — honesty stops being a rule about intent and becomes a rule about
//! what typechecks (ADR-0003 §1).
//!
//! The 11pm case still compiles: a detector that wants to claim "Python,
//! because the Dockerfile says `FROM python:3.11`" *can*, by citing that line.
//! But it must cite it, and having to write the citation is what makes the
//! author notice they are choosing a [`Confidence`] — which is what keeps the
//! claim off disk.
//!
//! [`Confidence`]: crate::model::Confidence

use std::path::Path;

use serde::Serialize;

/// A stable detector name. Appears in every [`Evidence`] and in every
/// conflict report, so "why do you think that?" always has an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct DetectorId(pub &'static str);

impl std::fmt::Display for DetectorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// How authoritative the *kind* of source is, independent of confidence.
///
/// Used only as a tie-break: a lockfile records what is actually installed, a
/// manifest records a range, and a range is a wish (ADR-0003 §5 step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Specificity {
    /// Inferred from something that is not about this fact at all.
    Heuristic,
    /// A config file for a tool that implies the fact.
    Config,
    /// The project's own manifest declares it.
    Manifest,
    /// A lockfile records what is actually resolved.
    Lockfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
// Tagged `kind`, not `source`: this enum is itself the `source` field of
// `Evidence`, and tagging it `source` produced `evidence.source.source` in
// the JSON, which reads as a mistake even though it was not.
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EvidenceSource {
    /// A path relative to the project root.
    File { path: String },
    /// The exact argv that was run.
    Command { argv: Vec<String> },
}

/// Where a value came from, precise enough to check by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Evidence {
    pub source: EvidenceSource,
    /// A JSON pointer, TOML path, or line number into the source.
    pub locator: String,
    /// The bytes that justified the value. Truncated — this is a citation, not
    /// a copy of the file.
    pub excerpt: String,
}

/// Excerpts are for a human reading a conflict report. A whole minified
/// bundle pasted into one would make the report useless and the cache large.
const MAX_EXCERPT: usize = 200;

fn truncate(s: impl Into<String>) -> String {
    let mut s = s.into();
    // Collapse whitespace so a multi-line excerpt stays one readable line.
    if s.contains('\n') {
        s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    if s.chars().count() > MAX_EXCERPT {
        let cut: String = s.chars().take(MAX_EXCERPT).collect();
        format!("{cut}…")
    } else {
        s
    }
}

impl Evidence {
    /// Cite a file. `locator` addresses the specific place inside it.
    ///
    /// Note there is no variant of this that omits `locator` or `excerpt`:
    /// "it's in package.json somewhere" is not a citation.
    pub fn from_file(path: &Path, locator: impl Into<String>, excerpt: impl Into<String>) -> Self {
        Self {
            // Always forward slashes: this string is compared in snapshots and
            // shown to users, and a citation that differs between Windows and
            // Linux would make every snapshot platform-specific.
            source: EvidenceSource::File {
                path: path.to_string_lossy().replace('\\', "/"),
            },
            locator: locator.into(),
            excerpt: truncate(excerpt),
        }
    }

    /// Cite a subprocess. `argv` is the exact command that produced `excerpt`.
    pub fn from_command(argv: &[String], excerpt: impl Into<String>) -> Self {
        Self {
            source: EvidenceSource::Command {
                argv: argv.to_vec(),
            },
            locator: String::new(),
            excerpt: truncate(excerpt),
        }
    }

    /// The cited path, if this evidence is a file.
    #[must_use]
    pub fn file_path(&self) -> Option<&str> {
        match &self.source {
            EvidenceSource::File { path } => Some(path),
            EvidenceSource::Command { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpts_are_truncated_and_flattened() {
        let long = "x".repeat(500);
        let e = Evidence::from_file(Path::new("a.json"), "$", long);
        assert!(e.excerpt.chars().count() <= MAX_EXCERPT + 1);
        assert!(e.excerpt.ends_with('…'));

        let multiline = Evidence::from_file(Path::new("a.json"), "$", "one\n  two\n\tthree");
        assert_eq!(multiline.excerpt, "one two three");
    }

    #[test]
    fn file_paths_are_normalised_so_snapshots_are_not_platform_specific() {
        let e = Evidence::from_file(Path::new("sub\\dir\\package.json"), "$", "x");
        assert_eq!(e.file_path(), Some("sub/dir/package.json"));
    }

    #[test]
    fn specificity_ranks_lockfiles_above_wishes() {
        assert!(Specificity::Lockfile > Specificity::Manifest);
        assert!(Specificity::Manifest > Specificity::Config);
        assert!(Specificity::Config > Specificity::Heuristic);
    }
}
