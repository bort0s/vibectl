//! The type that makes "never guess" a compile-time property.
//!
//! The obvious design is `Option<T>` plus a rule about not guessing. That
//! fails, and it fails predictably: `Option` has `unwrap_or_default`,
//! `unwrap_or`, `Default`, and a hundred other ergonomic paths from "I have
//! nothing" to "I have a value". At 11pm, in a detector that has found a
//! `Dockerfile` mentioning `python:3.11` and nothing else, every one of those
//! paths is a way to write a value the disk does not justify.
//!
//! So [`Detected`] implements **no** `Default`, `Deref`, `From<Option<T>>`, or
//! `unwrap_or*`. Consuming one requires a `match`. And [`Detected::known`]
//! cannot be called without an [`Evidence`], which itself has no constructor
//! that does not name a source. You cannot produce a detected value without
//! pointing at the bytes that justify it.
//!
//! See ADR-0003 §1.

use std::path::PathBuf;

use serde::Serialize;

use crate::detect::{DetectorId, Evidence};

/// How much the disk actually supports a value.
///
/// Three ordinal levels, not a float. A float invites `0.73` — fake precision,
/// and averaging two guesses produces a number that means nothing. Three levels
/// are defensible because each maps to a *different action*, which is what
/// makes this type load-bearing rather than decorative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Plausible, not corroborated. **Never written to a manifest** — surfaced
    /// as a suggestion the user can confirm. The gate lives once, in the merge
    /// pass, so no detector author can opt out of it.
    Weak,
    /// Strong circumstantial evidence, authoritative source absent. Written,
    /// and tagged as inferred so `--dry-run` shows it as such.
    Likely,
    /// The authoritative file for this fact states it directly.
    Certain,
}

impl Confidence {
    /// Whether a finding at this level may become a manifest value.
    ///
    /// The single place this question is answered. ADR-0003 §3.
    #[must_use]
    pub fn may_be_written(self) -> bool {
        self >= Confidence::Likely
    }
}

/// Why a field has no value.
///
/// A closed enum, because "why don't we know" is displayable information and
/// the whole point of the tool is to be able to answer it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UnknownReason {
    /// Nothing on disk spoke to this field.
    NoEvidence,
    /// Detectors disagreed and the tie-break ladder refused to pick. A coin
    /// flip is a guess.
    Conflict { candidates: Vec<Candidate> },
    /// Something was found, but only at [`Confidence::Weak`].
    LowConfidenceOnly { candidates: Vec<Candidate> },
    /// Present but unreadable — permissions, malformed JSON, too large.
    Unreadable { path: PathBuf, why: String },
    /// A detector ran out of time. It returns nothing rather than a partial
    /// guess.
    Timeout { detector: DetectorId },
    /// A detector could not run at all, e.g. `git` is not on `PATH`.
    NotAttempted { detector: DetectorId, why: String },
}

/// A value some detector proposed, kept with its justification so the user can
/// resolve a conflict by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Candidate {
    pub value: String,
    pub confidence: Confidence,
    pub detector: DetectorId,
    pub evidence: Evidence,
}

/// A field that either has a justified value or an explained absence.
///
/// Deliberately missing: `Default`, `Deref`, `From<Option<T>>`, `unwrap_or`,
/// `unwrap_or_default`, `unwrap_or_else`, and `or`. Their absence is the
/// mechanism, not an oversight — see the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[must_use]
pub enum Detected<T> {
    Known {
        value: T,
        confidence: Confidence,
        evidence: Evidence,
    },
    Unknown(UnknownReason),
}

impl<T> Detected<T> {
    /// The only constructor for a known value, and it cannot be called without
    /// evidence.
    pub fn known(value: T, confidence: Confidence, evidence: Evidence) -> Self {
        Detected::Known {
            value,
            confidence,
            evidence,
        }
    }

    pub fn unknown(reason: UnknownReason) -> Self {
        Detected::Unknown(reason)
    }

    /// Nothing on disk spoke to this field. The common case, and it is
    /// deliberately as easy to write as any other — the difficulty is meant to
    /// be on the *value* side, not here.
    pub fn no_evidence() -> Self {
        Detected::Unknown(UnknownReason::NoEvidence)
    }

    #[must_use]
    pub fn is_known(&self) -> bool {
        matches!(self, Detected::Known { .. })
    }

    /// Borrow the value if there is one.
    ///
    /// Note what this is *not*: there is no `unwrap_or`, so a caller that wants
    /// a fallback has to write the `match` and decide, in the open, what an
    /// absent value means. That is the intended friction.
    #[must_use]
    pub fn value(&self) -> Option<&T> {
        match self {
            Detected::Known { value, .. } => Some(value),
            Detected::Unknown(_) => None,
        }
    }

    #[must_use]
    pub fn reason(&self) -> Option<&UnknownReason> {
        match self {
            Detected::Known { .. } => None,
            Detected::Unknown(r) => Some(r),
        }
    }

    /// Whether this value may be written to a manifest.
    ///
    /// `Unknown` never may. `Weak` never may. Both answers come from here so
    /// there is one place to audit.
    #[must_use]
    pub fn may_be_written(&self) -> bool {
        match self {
            Detected::Known { confidence, .. } => confidence.may_be_written(),
            Detected::Unknown(_) => false,
        }
    }

    /// The value only if it is fit to write. This is what the manifest writer
    /// calls, and it is the choke point that keeps `Weak` off disk.
    #[must_use]
    pub fn writable_value(&self) -> Option<&T> {
        match self {
            Detected::Known {
                value, confidence, ..
            } if confidence.may_be_written() => Some(value),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{DetectorId, Evidence};
    use std::path::Path;

    fn ev() -> Evidence {
        Evidence::from_file(Path::new("package.json"), "$.engines.node", ">=22")
    }

    #[test]
    fn weak_values_are_never_writable() {
        let d = Detected::known("node@22".to_owned(), Confidence::Weak, ev());
        assert!(d.is_known(), "a weak finding is still a finding");
        assert_eq!(d.value(), Some(&"node@22".to_owned()));
        // ...but it cannot reach a manifest.
        assert!(!d.may_be_written());
        assert_eq!(d.writable_value(), None);
    }

    #[test]
    fn likely_and_certain_are_writable() {
        for c in [Confidence::Likely, Confidence::Certain] {
            let d = Detected::known("node@22".to_owned(), c, ev());
            assert!(d.may_be_written(), "{c:?} should be writable");
            assert!(d.writable_value().is_some());
        }
    }

    #[test]
    fn unknown_is_never_writable_whatever_the_reason() {
        let reasons = [
            UnknownReason::NoEvidence,
            UnknownReason::Conflict { candidates: vec![] },
            UnknownReason::LowConfidenceOnly { candidates: vec![] },
            UnknownReason::Unreadable {
                path: "x".into(),
                why: "bad json".into(),
            },
            UnknownReason::Timeout {
                detector: DetectorId("git"),
            },
        ];
        for r in reasons {
            let d: Detected<String> = Detected::unknown(r);
            assert!(!d.may_be_written());
            assert_eq!(d.writable_value(), None);
            assert_eq!(d.value(), None);
        }
    }

    #[test]
    fn confidence_orders_weak_below_likely_below_certain() {
        assert!(Confidence::Weak < Confidence::Likely);
        assert!(Confidence::Likely < Confidence::Certain);
        assert!(!Confidence::Weak.may_be_written());
        assert!(Confidence::Likely.may_be_written());
        assert!(Confidence::Certain.may_be_written());
    }
}
