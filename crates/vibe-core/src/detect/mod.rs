//! Detection: many mutually blind detectors, one merge pass.
//!
//! Detectors never read each other's output, never run conditionally on
//! another's result, and their execution order does not affect the outcome.
//! That is what makes a detector's unit test "fixture directory in,
//! `Vec<Finding>` out" with no orchestration and no mocking of peers
//! (ADR-0003 §4).
//!
//! No detector walks the filesystem. [`crate::walk::FileIndex`] is built once
//! per project and every detector is gated against it, so on a Go repository
//! the Node, Python and PHP detectors never run.

mod ctx;
pub mod detectors;
mod evidence;
mod merge;

pub use ctx::{DetectCtx, DetectError, ReadCache, is_forbidden_to_read};
pub use evidence::{DetectorId, Evidence, EvidenceSource, Specificity};
pub use merge::{Detection, Suggestion, merge};

use serde::Serialize;

use crate::model::Confidence;
use crate::walk::FileIndex;

/// A manifest field a detector can speak to.
///
/// An enum, not a string. Adding a field forces every `match` in the merge
/// pass to be reconsidered; a string key would let a new field fall silently
/// through a default branch, which is exactly how "never guess" erodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldPath {
    ProjectDescription,
    StackRuntime,
    StackFramework,
    StackService,
    RepoRemote,
    RepoVisibility,
    DeployUrl,
    DeployEnvRequired,
    GitBranch,
    GitLastCommit,
}

/// How findings for a field combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeKind {
    /// Exactly one true value exists, so disagreement is real.
    Scalar,
    /// Several values legitimately coexist; findings union.
    Set,
}

impl FieldPath {
    /// Every variant is listed explicitly rather than falling through a
    /// wildcard. A new field must be classified deliberately.
    #[must_use]
    pub fn merge_kind(self) -> MergeKind {
        match self {
            FieldPath::StackFramework | FieldPath::StackService | FieldPath::DeployEnvRequired => {
                MergeKind::Set
            }

            FieldPath::ProjectDescription
            | FieldPath::StackRuntime
            | FieldPath::RepoRemote
            | FieldPath::RepoVisibility
            | FieldPath::DeployUrl
            | FieldPath::GitBranch
            | FieldPath::GitLastCommit => MergeKind::Scalar,
        }
    }

    /// Every field, for exhaustive iteration in tests and reports.
    pub const ALL: [FieldPath; 10] = [
        FieldPath::ProjectDescription,
        FieldPath::StackRuntime,
        FieldPath::StackFramework,
        FieldPath::StackService,
        FieldPath::RepoRemote,
        FieldPath::RepoVisibility,
        FieldPath::DeployUrl,
        FieldPath::DeployEnvRequired,
        FieldPath::GitBranch,
        FieldPath::GitLastCommit,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum FieldValue {
    Text(String),
    Flag(bool),
}

impl FieldValue {
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            FieldValue::Text(s) => Some(s),
            FieldValue::Flag(_) => None,
        }
    }

    #[must_use]
    pub fn display(&self) -> String {
        match self {
            FieldValue::Text(s) => s.clone(),
            FieldValue::Flag(b) => b.to_string(),
        }
    }
}

/// One detector's claim about one field, with the citation that justifies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Finding {
    pub field: FieldPath,
    pub value: FieldValue,
    pub confidence: Confidence,
    /// Carried per finding rather than per detector, because one detector
    /// legitimately produces claims of different authority — `package.json`
    /// declaring `engines.node` is a manifest, while inferring `vite` from the
    /// presence of `vite.config.ts` is a config-level guess.
    ///
    /// This is a deliberate deviation from ADR-0003 §2, which put
    /// `specificity()` on the trait.
    pub specificity: Specificity,
    pub evidence: Evidence,
    pub detector: DetectorId,
}

/// What must exist before a detector is worth running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interest {
    FileName(&'static str),
    FileNamePattern {
        prefix: &'static str,
        suffix: &'static str,
    },
    DirName(&'static str),
}

impl Interest {
    fn matches(self, index: &FileIndex) -> bool {
        match self {
            Interest::FileName(n) => index.has_file(n),
            Interest::FileNamePattern { prefix, suffix } => {
                !index.root_files_matching(prefix, suffix).is_empty()
            }
            Interest::DirName(n) => index.has_dir(n),
        }
    }
}

pub trait Detector: Send + Sync {
    fn id(&self) -> DetectorId;

    /// What must be present for this detector to be worth running.
    fn interest(&self) -> &'static [Interest];

    /// Which fields this detector can speak to.
    ///
    /// Used to attribute a failure: if `Cargo.toml` is malformed, the merge
    /// pass reports `stack.runtime` as *unreadable* rather than as *no
    /// evidence*. The difference matters — one is a clean bill of health the
    /// tool has not earned, and the other is something the user can fix.
    fn produces(&self) -> &'static [FieldPath];

    /// Findings, or an explanation of why there are none.
    ///
    /// Returning `Ok(vec![])` means "I looked and found nothing to say", which
    /// is different from `Err(DetectError::Unreadable)` meaning "there was
    /// something here and I could not read it". The merge pass reports those
    /// differently, because the user can act on the second one.
    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError>;
}

/// The fixed v1 detector set. No plugin loading — see ADR-0003 §7.
#[must_use]
pub fn builtin_detectors() -> Vec<Box<dyn Detector>> {
    detectors::builtin()
}

/// Run every detector whose interest is satisfied, and merge the result.
///
/// Note the return type: [`Detection`], never a [`crate::WritePlan`]. Scanning
/// structurally cannot write, because the only function that writes takes a
/// plan and nothing on this path produces one.
pub fn run_detectors(
    detectors: &[Box<dyn Detector>],
    ctx: &DetectCtx<'_>,
) -> (Vec<Finding>, Vec<DetectorFailure>) {
    let mut findings = Vec::new();
    let mut failures = Vec::new();

    for detector in detectors {
        if !detector.interest().iter().any(|i| i.matches(ctx.files)) {
            continue;
        }
        if ctx.expired() {
            failures.push(DetectorFailure {
                detector: detector.id(),
                fields: detector.produces(),
                error: DetectError::Timeout,
            });
            continue;
        }
        match detector.detect(ctx) {
            Ok(mut f) => findings.append(&mut f),
            Err(error) => failures.push(DetectorFailure {
                detector: detector.id(),
                fields: detector.produces(),
                error,
            }),
        }
    }

    // Deterministic order regardless of detector iteration, so two scans of an
    // unchanged directory produce byte-identical results.
    findings.sort_by(|a, b| {
        a.field
            .cmp(&b.field)
            .then(b.confidence.cmp(&a.confidence))
            .then(b.specificity.cmp(&a.specificity))
            .then(a.detector.cmp(&b.detector))
            .then(a.value.display().cmp(&b.value.display()))
    });
    (findings, failures)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorFailure {
    pub detector: DetectorId,
    /// The fields this detector would have spoken to, so the failure can be
    /// attributed rather than showing up as an unexplained empty field.
    pub fields: &'static [FieldPath],
    pub error: DetectError,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_is_classified_and_none_falls_through() {
        // The point of the exhaustive match: this loop would need updating if
        // a field were added without being classified, because ALL is also
        // exhaustive and the compiler checks the match.
        let sets: Vec<_> = FieldPath::ALL
            .iter()
            .filter(|f| f.merge_kind() == MergeKind::Set)
            .collect();
        assert_eq!(sets.len(), 3, "frameworks, services, env_required");
    }
}
