//! The declared writer identity, validated as a path component.
//!
//! ADR-0011 §7a makes the identity **one field of a filename** —
//! `<session>__<agent>__<identity>.jsonl`, or `<session>__<identity>.jsonl`
//! when the payload carries no `agent_id` — so this is user-supplied configuration
//! reaching a path. That is path traversal in a value the user controls, and
//! it is the same class ADR-0005 §10 rule 4 handles for URLs, with the same
//! answer: a **closed allowlist**, because nobody writes down the traversal
//! form they have not met.
//!
//! # What was measured, and what the measurement changed
//!
//! Probed on **Windows 10 Pro 19045, node v24.16.0** (ADR-0011 §7a). The
//! device-name list — `CON`, `NUL`, `PRN`, `LPT*` — turned out **not** to be
//! the hazard: every one of them created an ordinary file on that build, and
//! in this design they are unreachable anyway, because the base name is never
//! exactly a device name. Recalling that list would have aimed the validation
//! at the wrong thing.
//!
//! The hazard the measurement *did* find is a **collision**, and it defeats a
//! uniqueness check that looks correct: Windows folds case and strips a
//! trailing dot or space, so `foo`, `foo·`, `foo.` and `Foo` are four distinct
//! declared strings and **one file**. Hence [`file_key`], and hence the rule
//! that uniqueness is compared on the key rather than on the declared string.
//!
//! # Two layers, and the second one is not redundant
//!
//! 1. [`WriterIdentity::parse`] rejects everything outside the charset. After
//!    it, a trailing dot or space is **unrepresentable** rather than merely
//!    detected — the technique ADR-0001 §3 uses for the missing
//!    `FileOp::Delete` and ADR-0005 §10 rule 1 uses for its closed enum.
//! 2. [`file_key`] still folds case and strips trailing dots and spaces,
//!    because the *contract read* (ADR-0011 §7a) enumerates identities from a
//!    config **before** anything is validated, and must report a duplicate as
//!    a duplicate. Layer 1 makes the dot case unreachable for accepted
//!    identities; layer 2 is what sees it in a config that was never accepted.
//!
//! **Consequence worth stating rather than discovering:** for a charset-valid
//! identity, case-folding is the *only* normalisation that can ever fire. The
//! trailing-dot half of the rule is dead code on the accept path by
//! construction, and alive on the config-inspection path. Those are different
//! paths and both are real.
//!
//! # The coarsest rule is applied on every platform, deliberately
//!
//! `foo` and `Foo` are two files on ext4 and one file on NTFS and on default
//! macOS. Applying the **coarsest** rule everywhere refuses some configurations
//! that would have worked on Linux, and never permits a twin writer anywhere. A
//! config that works on one developer's machine and silently twins on another's
//! is the cross-platform failure this project keeps declining to ship; being
//! over-strict is the direction that fails safe.

use std::collections::BTreeMap;

use serde::Serialize;

/// Longest accepted declared identity, in bytes.
///
/// **This number is path arithmetic, not taste.** The longest filename is
/// `<session>__<agent>__<identity>.jsonl`, and Windows resolves a non-extended
/// path against `MAX_PATH` = 260. With [`SESSION_MAX_LEN`] at 64 and
/// [`AGENT_MAX_LEN`] at 40 it tops out at 64 + 2 + 40 + 2 + 48 + 6 = **162
/// characters**, leaving ~98 for the sink directory — more than the ~45 a
/// `LocalAppData` sink needs.
///
/// *The third component was added 2026-08-18 and spent 42 characters of the
/// headroom the two-part key had.*
///
/// Raising either constant spends that headroom, and the failure it buys is
/// `path too long` at write time on one platform only.
pub const IDENTITY_MAX_LEN: usize = 48;

/// The filename's field separator.
///
/// Forbidden inside every component, which is what lets the component **count**
/// distinguish a session-level record from an agent-level one without reserving
/// a literal for *"no agent"*.
pub const SEPARATOR: &str = "__";

/// Longest accepted agent component, in bytes.
///
/// Observed `agent_id` values on Claude Code 2.1.233 are **17 lowercase
/// alphanumerics** (`ab8b50189992e6091`). That is a **sample of seven, not a
/// guarantee about the field**, so the bound is headroom rather than a fit, and
/// a longer or out-of-charset value is refused rather than assumed impossible.
pub const AGENT_MAX_LEN: usize = 40;

/// Longest accepted session component, in bytes. See [`IDENTITY_MAX_LEN`] for
/// the arithmetic. Claude Code session ids are UUIDs (36 characters) on
/// 2.1.233; 64 is headroom, and the version is part of that claim.
pub const SESSION_MAX_LEN: usize = 64;

/// Why a string was refused as a path component.
///
/// Carries **data, never a remediation sentence** — ADR-0001 §4. `vibectl` and
/// any future frontend write their own prose; what crosses the boundary is the
/// reason and the offending position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "rejection", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ComponentRejection {
    /// The declared value was empty.
    Empty,
    /// Longer than the bound. `max` is carried so a frontend need not know it.
    TooLong { len: usize, max: usize },
    /// A byte outside the allowlist, with where it sat.
    ///
    /// The byte is reported as a number rather than as a character because a
    /// non-UTF-8 or non-printing byte has no character to show, and rendering
    /// one would be inventing a value.
    IllegalByte { index: usize, byte: u8 },
    /// Contains `__`, which is the filename's field separator.
    ///
    /// Single `_` is fine; two in a row are not. This is what lets the
    /// **component count** distinguish a session-level record
    /// (`<session>__<identity>`) from an agent-level one
    /// (`<session>__<agent>__<identity>`) without reserving a word for
    /// *"no agent"* — a reserved literal such as `root` could collide with a
    /// real `agent_id`, and nothing measured bounds that id space.
    ContainsSeparator { index: usize },
}

impl ComponentRejection {
    /// Stable key, safe to branch on and to print as data.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            ComponentRejection::Empty => "empty",
            ComponentRejection::TooLong { .. } => "too_long",
            ComponentRejection::IllegalByte { .. } => "illegal_byte",
            ComponentRejection::ContainsSeparator { .. } => "contains_separator",
        }
    }
}

/// A declared writer identity that has been validated as a path component.
///
/// The type is the enforcement: a `WriterIdentity` cannot be constructed from
/// an unvalidated string, so a function taking one cannot be passed a
/// traversal. Skipping the check requires changing a signature, which is a
/// visible diff rather than an omission — the same enforcement ADR-0005 §10
/// rule 4 gets from [`crate::GitUrl`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct WriterIdentity {
    declared: String,
}

impl WriterIdentity {
    /// Validate a declared identity.
    ///
    /// The allowlist is ASCII alphanumerics, `-` and `_`. Closed, not a
    /// denylist: `..`, `/`, `\`, `:`, `*`, `?`, `|`, `<`, `>`, `"`, a trailing
    /// dot and a trailing space are all rejected **by not being on it**, rather
    /// than by being enumerated as hazards.
    ///
    /// # Errors
    ///
    /// [`ComponentRejection`] naming what was wrong and where.
    pub fn parse(declared: &str) -> Result<Self, ComponentRejection> {
        validate_component(declared, IDENTITY_MAX_LEN)?;
        Ok(Self {
            declared: declared.to_owned(),
        })
    }

    /// The identity exactly as declared.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.declared
    }

    /// The key two declared identities are compared on for uniqueness.
    ///
    /// **Not the declared string.** The filesystem's notion of *same file* is
    /// coarser than string equality, and the check must use the filesystem's,
    /// because that is the one that decides whether two writers share a file.
    #[must_use]
    pub fn file_key(&self) -> String {
        file_key(&self.declared)
    }
}

/// A session component that has been validated as a path component.
///
/// Separate from [`WriterIdentity`] because it has a different bound and a
/// different origin — the identity is declared by the hook, the session comes
/// from the agent's payload — and because a function that wants one must not
/// silently accept the other.
///
/// **ADR-0011 §7a validates the identity as a path component and is silent
/// about the session**, which is the other half of the same filename and is
/// equally data reaching a path. Validated here on the same rule; recorded as a
/// gap in the spec rather than as an extension of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SessionComponent {
    declared: String,
}

impl SessionComponent {
    /// Validate a session id as a path component.
    ///
    /// # Errors
    ///
    /// [`ComponentRejection`] naming what was wrong and where.
    pub fn parse(declared: &str) -> Result<Self, ComponentRejection> {
        validate_component(declared, SESSION_MAX_LEN)?;
        Ok(Self {
            declared: declared.to_owned(),
        })
    }

    /// The session id exactly as it arrived in the payload.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.declared
    }
}

/// An agent component that has been validated as a path component.
///
/// **Absent on parent-level events**, which is why every function taking one
/// takes an `Option`. ADR-0011 §7a encodes that absence by the component
/// *count* rather than by a reserved word: a literal such as `root` could
/// collide with a real `agent_id`, and nothing measured bounds that id space.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct AgentComponent {
    declared: String,
}

impl AgentComponent {
    /// Validate an `agent_id` as a path component.
    ///
    /// # Errors
    ///
    /// [`ComponentRejection`] naming what was wrong and where.
    pub fn parse(declared: &str) -> Result<Self, ComponentRejection> {
        validate_component(declared, AGENT_MAX_LEN)?;
        Ok(Self {
            declared: declared.to_owned(),
        })
    }

    /// The agent id exactly as it arrived in the payload.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.declared
    }
}

/// The one charset check, shared so the three components cannot drift apart.
fn validate_component(declared: &str, max: usize) -> Result<(), ComponentRejection> {
    if declared.is_empty() {
        return Err(ComponentRejection::Empty);
    }
    if declared.len() > max {
        return Err(ComponentRejection::TooLong {
            len: declared.len(),
            max,
        });
    }
    for (index, byte) in declared.bytes().enumerate() {
        if !(byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_') {
            return Err(ComponentRejection::IllegalByte { index, byte });
        }
    }
    // `__` is the filename's field separator, so no component may contain it.
    // That is what lets the component COUNT carry "is there an agent" without
    // reserving a literal for "no agent" — see `ContainsSeparator`.
    if let Some(index) = declared.find(SEPARATOR) {
        return Err(ComponentRejection::ContainsSeparator { index });
    }
    Ok(())
}

/// Fold a declared string to the key the filesystem would collide on.
///
/// Case-folded, with trailing dots and spaces stripped — the three
/// transformations measured on Windows 10 Pro 19045 (ADR-0011 §7a).
///
/// **Takes a raw `&str` on purpose.** The contract read enumerates identities
/// out of a settings file before any of them is known to be valid, and it must
/// be able to report *"these two collide"* about strings this crate would
/// refuse. Folding only ASCII case is exact for anything
/// [`WriterIdentity::parse`] accepts, and partial for anything it would not —
/// which is sound here, because the invalid ones are rejected on the charset
/// anyway and never reach a filename.
#[must_use]
pub fn file_key(declared: &str) -> String {
    declared.trim_end_matches(['.', ' ']).to_ascii_lowercase()
}

/// Two or more declared identities that resolve to one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IdentityCollision {
    /// The shared [`file_key`].
    pub file_key: String,
    /// Every declared string that folded to it, in the order given.
    pub declared: Vec<String>,
}

/// Find declared identities that would share a file.
///
/// This is the check ADR-0011 §7a puts at **`vibe monitor install`** and at the
/// **contract read**, and it is deliberately a function over declared strings
/// rather than a scan of written records: a duplicated identity is a
/// *configuration* fact, fixed before any event fires, so it is decidable
/// without reading a single record. Inspecting records notices the collision
/// after both writers have written and after the damage.
///
/// **The writer cannot be the detector.** Creating the file exclusively catches
/// nothing — a writer appends across every event of a session, so from the
/// second event onward the file legitimately exists and is indistinguishable
/// from a twin's.
#[must_use]
pub fn collisions(declared: &[String]) -> Vec<IdentityCollision> {
    let mut by_key: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in declared {
        by_key.entry(file_key(d)).or_default().push(d.clone());
    }
    by_key
        .into_iter()
        .filter(|(_, group)| group.len() > 1)
        .map(|(file_key, declared)| IdentityCollision { file_key, declared })
        .collect()
}
