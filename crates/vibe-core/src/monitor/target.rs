//! Where `vibe monitor install` may write — a closed set with one member.
//!
//! ADR-0011 §7b's containment argument, as a type. ADR-0005 §10 rule 5 admits a
//! write whose deepest existing ancestor canonicalises inside **a configured
//! root or the plan's declared target directory**, and the user-level settings
//! file is in neither. Widening the rule to admit "the home directory" would
//! trade a bounded invariant for an unbounded one, so the route is a **closed
//! variant** instead: the op names no path and carries no choice, and `apply`
//! resolves the one target itself.
//!
//! # The invariant is the CARDINALITY, not the member
//!
//! §9 is explicit about this and it is worth restating where the type lives: a
//! control asserting *"this write lands at the user settings file"* passes just
//! as happily with a second variant sitting beside it. §7b's first draft was
//! two-valued — user or project — while project-level install had already been
//! closed by the one-sink decision, so a representable state existed that no
//! command could produce, and the moment anything reached it all three of
//! `read_sink`'s N-sink problems come back.
//!
//! So the assertion is on the size of this set. See
//! `the_settings_target_set_has_exactly_one_member`.
//!
//! # What makes this write safe is a property of the path, not a permission
//!
//! The target has **no component derived from data**. Not from a payload, not
//! from a scan, not from a project name, not from anything a user typed: it is
//! the home directory plus two fixed literals. The traversal class rule 5 exists
//! to stop is a path *assembled* from values, and nothing is assembled here — so
//! the hazard does not arise rather than being checked for.
//!
//! Compare [`super::identity`], whose filename has three payload-or-argv
//! components reaching a path and is charset-validated at both ends precisely
//! because it does.
//!
//! **Rule 5 still runs anyway**, against the resolved path, because a check
//! skipped on the ground that it cannot fail is a check that stops running when
//! the ground moves.
//!
//! # Deliberately NOT `#[non_exhaustive]`
//!
//! Every other public enum in this crate carries it, and this one must not:
//! `#[non_exhaustive]` tells a downstream crate the set may grow, which is the
//! opposite of what this type asserts. Closedness is the property being
//! shipped.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The directory Claude Code keeps its configuration in, under either root.
///
/// Two components rather than one string, so the separator is the platform's
/// and never a literal `/` in a path — the same reason [`super::super::prompts`]
/// splits its own.
pub const CLAUDE_DIR: &str = ".claude";

/// The file install edits.
pub const SETTINGS_FILE: &str = "settings.json";

/// The settings file an install writes to.
///
/// One member. See the module docs for why that is the invariant rather than an
/// implementation detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsTarget {
    /// `<home>/.claude/settings.json`.
    User,
}

impl SettingsTarget {
    /// Every member of the set.
    ///
    /// **Hand-written, with the compiler forcing the hand.** A new variant does
    /// not update this array by itself — nothing in Rust can do that — but it
    /// cannot be added without breaking the exhaustive matches in [`Self::key`],
    /// [`Self::file_name`] and the control, each of which stops compiling until
    /// an arm exists. The author who adds those arms is the author who has to
    /// decide whether to add the member here, and adding it turns
    /// `the_settings_target_set_has_exactly_one_member` red.
    ///
    /// That chain is what §9 asks for: re-opening project-level install becomes
    /// a decision that breaks something rather than a diff that does not.
    pub const ALL: &'static [SettingsTarget] = &[SettingsTarget::User];

    /// Stable key, and one of the exhaustive matches guarding [`Self::ALL`].
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            SettingsTarget::User => "user",
        }
    }

    /// The file name under the containment root.
    ///
    /// A fixed literal per variant, which is the half of the containment
    /// argument this type owns: there is no parameter here for a caller to put
    /// a path in.
    #[must_use]
    pub fn file_name(self) -> &'static str {
        match self {
            SettingsTarget::User => SETTINGS_FILE,
        }
    }

    /// The directory that bounds the write, given a home directory.
    ///
    /// **`home` is passed in rather than resolved here so a test can plant
    /// one** — the precedent is [`crate::prompts::list_prompts`], which takes
    /// `user_home` for exactly this reason. A resolver buried in here would
    /// make every control below depend on the real user's configuration.
    #[must_use]
    pub fn containment_root(self, home: &Path) -> PathBuf {
        match self {
            SettingsTarget::User => home.join(CLAUDE_DIR),
        }
    }

    /// The file itself.
    #[must_use]
    pub fn resolve(self, home: &Path) -> PathBuf {
        self.containment_root(home).join(self.file_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The set has exactly ONE member** (ADR-0011 §7b, §9).
    ///
    /// The assertion that matters is the count, not the member. A control
    /// asserting only *"the write lands at the user settings file"* stays green
    /// with a second variant beside it, and the second variant is the whole
    /// hazard: project-level install was closed by the one-sink decision, so a
    /// representable state with no producer is a state nothing is guarding.
    ///
    /// The match below is what makes this bite. It is exhaustive, so adding a
    /// variant stops this file compiling — the author cannot reach the
    /// assertion without first deciding to add an arm, and the assertion is
    /// then waiting for them.
    #[test]
    fn the_settings_target_set_has_exactly_one_member() {
        for target in SettingsTarget::ALL {
            match target {
                SettingsTarget::User => {}
            }
        }

        assert_eq!(
            SettingsTarget::ALL.len(),
            1,
            "the settings-target set has grown. That is not a diff to wave \
             through: ADR-0011 §7b closed project-level install because \
             `read_sink` takes one directory and no caller merges — two sinks \
             break `sequencing`, change what `identity_collisions` means, and \
             leave one unreadable sink among N with no representation. \
             Re-opening it is a decision to take in the ADR first."
        );
    }

    /// The target names no path component that came from anywhere.
    ///
    /// Asserted on the produced path rather than on the source, because the
    /// claim is about what reaches the filesystem: every component below the
    /// home directory is a compile-time literal.
    #[test]
    fn the_resolved_path_is_the_home_plus_two_literals() {
        let home = Path::new("/somewhere/home");
        let resolved = SettingsTarget::User.resolve(home);

        assert_eq!(resolved, home.join(CLAUDE_DIR).join(SETTINGS_FILE));
        assert!(resolved.starts_with(SettingsTarget::User.containment_root(home)));

        // The paired half: the containment root is a real prefix, so the file
        // cannot sit outside what bounds it. Without this the assertion above
        // is satisfied by a root equal to the file.
        assert_ne!(resolved, SettingsTarget::User.containment_root(home));
    }
}
