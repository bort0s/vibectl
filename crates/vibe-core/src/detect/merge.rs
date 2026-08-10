//! Resolving many findings into one honest answer per field.
//!
//! Two rules live here and nowhere else, so there is one place to audit:
//!
//! 1. **A `Weak` finding can never become a manifest value**, no matter what a
//!    detector author intended.
//! 2. **A tie that cannot be broken becomes `Unknown`, not a choice.** A coin
//!    flip is a guess, and the tool's one differentiating feature is not
//!    lying.
//!
//! ADR-0003 §3 and §5.

use serde::Serialize;

use crate::detect::{
    DetectError, DetectorFailure, DetectorId, Evidence, FieldPath, FieldValue, Finding, MergeKind,
};
use crate::model::{Candidate, Confidence, Detected, UnknownReason};

/// A value that was found but deliberately not written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Suggestion {
    pub field: FieldPath,
    pub value: String,
    pub confidence: Confidence,
    pub detector: DetectorId,
    pub evidence: Evidence,
    pub why: SuggestionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SuggestionReason {
    /// Below the confidence bar for writing. Offered for the user to confirm.
    LowConfidence,
    /// One side of a conflict the ladder refused to resolve.
    Conflict,
}

/// Everything detection concluded about one project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Detection {
    pub description: Detected<String>,
    pub runtime: Detected<String>,
    pub frameworks: Vec<String>,
    pub services: Vec<String>,
    pub env_required: Vec<String>,
    pub remote: Detected<String>,
    pub visibility: Detected<String>,
    pub deploy_url: Detected<String>,
    pub branch: Detected<String>,
    pub last_commit: Detected<String>,
    /// Found, but not written: weak values and unresolved conflicts.
    pub suggestions: Vec<Suggestion>,
    /// Sources that exist but could not be read.
    ///
    /// Reported whether or not the affected fields ended up with a value. A
    /// failure that is only consulted when a field is otherwise empty produces
    /// a perverse result: corrupting one of two competing manifests removes it
    /// from the conflict, and an honest `Unknown{Conflict}` silently becomes a
    /// confident single answer. Breaking a file must never *raise* the tool's
    /// confidence.
    pub unreadable: Vec<UnreadableSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct UnreadableSource {
    pub detector: DetectorId,
    pub path: String,
    pub why: String,
    /// The fields this failure leaves less certain than they appear.
    pub affects: Vec<FieldPath>,
}

pub fn merge(findings: &[Finding], failures: &[DetectorFailure]) -> Detection {
    let mut suggestions = Vec::new();

    let mut scalar = |field: FieldPath| resolve_scalar(field, findings, failures, &mut suggestions);

    let description = scalar(FieldPath::ProjectDescription);
    let runtime = scalar(FieldPath::StackRuntime);
    let remote = scalar(FieldPath::RepoRemote);
    let visibility = scalar(FieldPath::RepoVisibility);
    let deploy_url = scalar(FieldPath::DeployUrl);
    let branch = scalar(FieldPath::GitBranch);
    let last_commit = scalar(FieldPath::GitLastCommit);
    let frameworks = resolve_set(FieldPath::StackFramework, findings, &mut suggestions);
    let services = resolve_set(FieldPath::StackService, findings, &mut suggestions);
    let env_required = resolve_set(FieldPath::DeployEnvRequired, findings, &mut suggestions);

    let mut unreadable: Vec<UnreadableSource> = failures
        .iter()
        .filter_map(|f| match &f.error {
            DetectError::Unreadable { path, why } => Some(UnreadableSource {
                detector: f.detector,
                path: path.clone(),
                why: why.clone(),
                affects: f.fields.to_vec(),
            }),
            _ => None,
        })
        .collect();
    unreadable.sort_by(|a, b| a.path.cmp(&b.path).then(a.detector.cmp(&b.detector)));

    suggestions.sort_by(|a, b| {
        a.field
            .cmp(&b.field)
            .then(a.value.cmp(&b.value))
            .then(a.detector.cmp(&b.detector))
    });
    suggestions.dedup();

    Detection {
        description,
        runtime,
        frameworks,
        services,
        env_required,
        remote,
        visibility,
        deploy_url,
        branch,
        last_commit,
        suggestions,
        unreadable,
    }
}

/// Union, dedupe, sort.
///
/// Sorting is not cosmetic: it keeps `insta` snapshots stable and stops
/// `vibe sync` producing a diff on every run purely from iteration order.
fn resolve_set(
    field: FieldPath,
    findings: &[Finding],
    suggestions: &mut Vec<Suggestion>,
) -> Vec<String> {
    debug_assert_eq!(field.merge_kind(), MergeKind::Set);

    let mut values = Vec::new();
    for f in findings.iter().filter(|f| f.field == field) {
        let text = f.value.display();
        if f.confidence.may_be_written() {
            values.push(text);
        } else {
            suggestions.push(Suggestion {
                field,
                value: text,
                confidence: f.confidence,
                detector: f.detector,
                evidence: f.evidence.clone(),
                why: SuggestionReason::LowConfidence,
            });
        }
    }
    values.sort();
    values.dedup();
    values
}

fn resolve_scalar(
    field: FieldPath,
    findings: &[Finding],
    failures: &[DetectorFailure],
    suggestions: &mut Vec<Suggestion>,
) -> Detected<String> {
    debug_assert_eq!(field.merge_kind(), MergeKind::Scalar);

    let all: Vec<&Finding> = findings.iter().filter(|f| f.field == field).collect();

    if all.is_empty() {
        // Distinguish "nothing on disk spoke to this" from "something did and
        // we could not read it". The second is actionable; presenting it as
        // the first would be a clean bill of health the tool has not earned.
        if let Some(reason) = attributed_failure(field, failures) {
            return Detected::unknown(reason);
        }
        return Detected::no_evidence();
    }

    // Step 1: drop Weak. If that was everything, the field is unknown and the
    // weak values are offered as suggestions rather than silently discarded.
    let (strong, weak): (Vec<&Finding>, Vec<&Finding>) =
        all.iter().partition(|f| f.confidence.may_be_written());

    for f in &weak {
        suggestions.push(Suggestion {
            field,
            value: f.value.display(),
            confidence: f.confidence,
            detector: f.detector,
            evidence: f.evidence.clone(),
            why: SuggestionReason::LowConfidence,
        });
    }

    if strong.is_empty() {
        return Detected::unknown(UnknownReason::LowConfidenceOnly {
            candidates: weak.iter().map(|f| candidate(f)).collect(),
        });
    }

    // Step 2: higher confidence wins outright.
    let best_confidence = strong
        .iter()
        .map(|f| f.confidence)
        .max()
        .unwrap_or(Confidence::Likely);
    let strong: Vec<&Finding> = strong
        .into_iter()
        .filter(|f| f.confidence == best_confidence)
        .collect();

    // Step 3: tie on confidence, higher specificity wins. A lockfile records
    // what is installed; a manifest range records a wish.
    let best_specificity = strong.iter().map(|f| f.specificity).max();
    let finalists: Vec<&Finding> = strong
        .into_iter()
        .filter(|f| Some(f.specificity) == best_specificity)
        .collect();

    let mut distinct: Vec<String> = finalists.iter().map(|f| f.value.display()).collect();
    distinct.sort();
    distinct.dedup();

    // Step 4: still tied and the values differ — we do not pick.
    if distinct.len() > 1 {
        for f in &finalists {
            suggestions.push(Suggestion {
                field,
                value: f.value.display(),
                confidence: f.confidence,
                detector: f.detector,
                evidence: f.evidence.clone(),
                why: SuggestionReason::Conflict,
            });
        }
        return Detected::unknown(UnknownReason::Conflict {
            candidates: finalists.iter().map(|f| candidate(f)).collect(),
        });
    }

    let winner = finalists
        .first()
        .expect("non-empty by construction: `strong` was checked above");
    Detected::known(
        winner.value.display(),
        winner.confidence,
        winner.evidence.clone(),
    )
}

fn candidate(f: &Finding) -> Candidate {
    Candidate {
        value: f.value.display(),
        confidence: f.confidence,
        detector: f.detector,
        evidence: f.evidence.clone(),
    }
}

/// If a detector that would have spoken to this field failed, say so.
fn attributed_failure(field: FieldPath, failures: &[DetectorFailure]) -> Option<UnknownReason> {
    let failure = failures.iter().find(|f| f.fields.contains(&field))?;
    Some(match &failure.error {
        DetectError::Unreadable { path, why } => UnknownReason::Unreadable {
            path: path.into(),
            why: why.clone(),
        },
        DetectError::Timeout => UnknownReason::Timeout {
            detector: failure.detector,
        },
        DetectError::NotAttempted { why } => UnknownReason::NotAttempted {
            detector: failure.detector,
            why: why.clone(),
        },
    })
}

/// Helper for detectors and tests: a text finding.
#[must_use]
pub(crate) fn text_finding(
    field: FieldPath,
    value: impl Into<String>,
    confidence: Confidence,
    specificity: crate::detect::Specificity,
    evidence: Evidence,
    detector: DetectorId,
) -> Finding {
    Finding {
        field,
        value: FieldValue::Text(value.into()),
        confidence,
        specificity,
        evidence,
        detector,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Specificity;
    use std::path::Path;

    fn ev(file: &str) -> Evidence {
        Evidence::from_file(Path::new(file), "$", "x")
    }

    fn f(
        field: FieldPath,
        value: &str,
        confidence: Confidence,
        specificity: Specificity,
        detector: &'static str,
    ) -> Finding {
        text_finding(
            field,
            value,
            confidence,
            specificity,
            ev(detector),
            DetectorId(detector),
        )
    }

    #[test]
    fn a_lone_weak_finding_never_becomes_a_value() {
        let findings = vec![f(
            FieldPath::StackRuntime,
            "python@3.11",
            Confidence::Weak,
            Specificity::Heuristic,
            "dockerfile",
        )];
        let d = merge(&findings, &[]);

        assert!(!d.runtime.is_known(), "weak must not become a value");
        assert!(matches!(
            d.runtime.reason(),
            Some(UnknownReason::LowConfidenceOnly { .. })
        ));
        // But it is not thrown away — the user can confirm it.
        assert_eq!(d.suggestions.len(), 1);
        assert_eq!(d.suggestions[0].value, "python@3.11");
        assert_eq!(d.suggestions[0].why, SuggestionReason::LowConfidence);
    }

    #[test]
    fn certain_beats_likely() {
        let findings = vec![
            f(
                FieldPath::StackRuntime,
                "node@20",
                Confidence::Likely,
                Specificity::Config,
                "a",
            ),
            f(
                FieldPath::StackRuntime,
                "node@22",
                Confidence::Certain,
                Specificity::Manifest,
                "b",
            ),
        ];
        let d = merge(&findings, &[]);
        assert_eq!(d.runtime.value().map(String::as_str), Some("node@22"));
    }

    #[test]
    fn a_lockfile_outranks_a_manifest_range_at_equal_confidence() {
        let findings = vec![
            f(
                FieldPath::StackRuntime,
                "node@20",
                Confidence::Certain,
                Specificity::Manifest,
                "pkg",
            ),
            f(
                FieldPath::StackRuntime,
                "node@22.11.0",
                Confidence::Certain,
                Specificity::Lockfile,
                "lock",
            ),
        ];
        let d = merge(&findings, &[]);
        assert_eq!(
            d.runtime.value().map(String::as_str),
            Some("node@22.11.0"),
            "a lockfile records what is installed; a range records a wish"
        );
    }

    #[test]
    fn an_unbreakable_tie_becomes_unknown_rather_than_a_coin_flip() {
        let findings = vec![
            f(
                FieldPath::StackRuntime,
                "node@22",
                Confidence::Certain,
                Specificity::Manifest,
                "node",
            ),
            f(
                FieldPath::StackRuntime,
                "python@3.12",
                Confidence::Certain,
                Specificity::Manifest,
                "py",
            ),
        ];
        let d = merge(&findings, &[]);

        assert!(
            !d.runtime.is_known(),
            "a polyglot repo must not get a guess"
        );
        let Some(UnknownReason::Conflict { candidates }) = d.runtime.reason() else {
            panic!("expected a conflict, got {:?}", d.runtime.reason());
        };
        assert_eq!(candidates.len(), 2);
        // Both sides are reported with their evidence so a human can resolve it.
        assert_eq!(d.suggestions.len(), 2);
        assert!(
            d.suggestions
                .iter()
                .all(|s| s.why == SuggestionReason::Conflict)
        );
    }

    #[test]
    fn identical_values_from_two_detectors_are_not_a_conflict() {
        let findings = vec![
            f(
                FieldPath::StackRuntime,
                "node@22",
                Confidence::Certain,
                Specificity::Manifest,
                "a",
            ),
            f(
                FieldPath::StackRuntime,
                "node@22",
                Confidence::Certain,
                Specificity::Manifest,
                "b",
            ),
        ];
        let d = merge(&findings, &[]);
        assert_eq!(d.runtime.value().map(String::as_str), Some("node@22"));
        assert!(d.suggestions.is_empty());
    }

    #[test]
    fn set_fields_union_and_sort_and_exclude_weak() {
        let findings = vec![
            f(
                FieldPath::StackFramework,
                "react@19",
                Confidence::Certain,
                Specificity::Manifest,
                "a",
            ),
            f(
                FieldPath::StackFramework,
                "vite",
                Confidence::Likely,
                Specificity::Config,
                "b",
            ),
            f(
                FieldPath::StackFramework,
                "react@19",
                Confidence::Certain,
                Specificity::Manifest,
                "c",
            ),
            f(
                FieldPath::StackFramework,
                "jquery",
                Confidence::Weak,
                Specificity::Heuristic,
                "d",
            ),
        ];
        let d = merge(&findings, &[]);
        assert_eq!(d.frameworks, vec!["react@19", "vite"], "sorted and deduped");
        assert!(
            d.suggestions.iter().any(|s| s.value == "jquery"),
            "the weak one is offered, not written"
        );
    }

    #[test]
    fn nothing_found_is_no_evidence_not_a_default() {
        let d = merge(&[], &[]);
        assert_eq!(d.runtime.reason(), Some(&UnknownReason::NoEvidence));
        assert!(d.frameworks.is_empty());
    }

    #[test]
    fn an_unreadable_source_is_reported_as_unreadable_not_as_no_evidence() {
        let failures = vec![DetectorFailure {
            detector: DetectorId("cargo"),
            fields: &[FieldPath::StackRuntime, FieldPath::ProjectDescription],
            error: DetectError::Unreadable {
                path: "Cargo.toml".to_owned(),
                why: "expected `=`".to_owned(),
            },
        }];
        let d = merge(&[], &failures);

        let Some(UnknownReason::Unreadable { path, why }) = d.runtime.reason() else {
            panic!("expected Unreadable, got {:?}", d.runtime.reason());
        };
        assert_eq!(path.to_string_lossy(), "Cargo.toml");
        assert!(why.contains("expected"));

        // A field that detector never claimed is still plain no-evidence.
        assert_eq!(d.deploy_url.reason(), Some(&UnknownReason::NoEvidence));
    }

    #[test]
    fn merging_is_deterministic_regardless_of_input_order() {
        let a = f(
            FieldPath::StackFramework,
            "vite",
            Confidence::Likely,
            Specificity::Config,
            "a",
        );
        let b = f(
            FieldPath::StackFramework,
            "react@19",
            Confidence::Certain,
            Specificity::Manifest,
            "b",
        );

        let one = merge(&[a.clone(), b.clone()], &[]);
        let two = merge(&[b, a], &[]);
        assert_eq!(one, two);
    }
}
