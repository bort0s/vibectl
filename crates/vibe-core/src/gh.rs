//! The closed set of `gh` invocations this crate is allowed to make.
//!
//! [`crate::git::GitOp`]'s argument, one program further along. ADR-0005 §10
//! says `gh` is *worse* than `git`, and says why: `gh alias set` and
//! `gh extension install` execute arbitrary binaries **by design**. That is the
//! documented feature, not an abuse of it, and it is the reason a
//! `{git, gh}` program allowlist closes nothing.
//!
//! What closes it is rule 1: the allowlist keys on the `(program, subcommand)`
//! pair. This enum has exactly one pair — `("gh", "repo create")` — and no
//! variant with anywhere to put a second one. `alias set` and
//! `extension install` are not rejected here; they are **unexpressible**, in the
//! same way [`crate::plan::FileOp`] has no `Delete` and [`crate::git::GitOp`]
//! has no `Push`.
//!
//! ## The one user string, and where it is allowed to land
//!
//! `--source=.` is a literal and the directory travels as the process's working
//! directory, not as an argument. `--push` and the visibility flag are literals.
//! So the only value from outside this crate that reaches argv is the repository
//! name, it is last, and it is separated by `--`.
//!
//! `cobra` resolves subcommands from the leading positional arguments *before*
//! flag parsing, so a name that happens to read like a subcommand cannot become
//! one from that position. The `--` is the second lock on the same door: it
//! stops a name beginning with `-` being read as a flag.
//!
//! ## Deviation from [`crate::git`]'s narrowing, chosen rather than inherited
//!
//! `GitOp`'s argument check is deliberately un-`--`-aware: it refuses a path
//! literally named `--exec-path=x` even though the separator would have made it
//! safe. That narrowing costs nothing there, because `Add` is only handed paths
//! this crate constructs.
//!
//! Here the guarded slot is by definition a string the *user* chose — the
//! project name — so the same rule would refuse to create a repository named
//! `alias`. That is a validation rule wearing a containment rule's clothes, and
//! ADR-0005 §10's own framing rejects it: rule 2 exists to catch a value landing
//! in a slot that turns out to be flag-parsed, and a slot after `--` in a
//! `cobra` command is not one. So [`crate::exec`]'s `gh` check scans up to the
//! separator and treats what follows as data.
//!
//! The check is an **allowlist** rather than a denylist in exchange: every
//! element before the `--` must be either the allowlisted subcommand pair or a
//! flag on a fixed list. A future variant that threads a user string into a
//! pre-separator slot fails closed, loudly, rather than being waved through by a
//! denylist nobody remembered to extend.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The program. A literal, and the only one this module names.
pub const GH_PROGRAM: &str = "gh";

/// The visibility a created repository is given.
///
/// Deliberately **not** [`crate::Visibility`], which carries an
/// `Other(String)` so a value written by a future build round-trips through a
/// manifest. That variant is right for a field being read and wrong for one
/// being turned into a command-line flag: `Other(s)` would put a user-chosen
/// string one `format!` away from argv. Two variants, both literals, no third
/// case.
///
/// There is no default. `gh` requires the choice to be stated, and picking one
/// on the user's behalf would mean this tool deciding whether their code is
/// published — the same class of invention as writing a plausible value into a
/// manifest field nothing detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoVisibility {
    Public,
    Private,
}

impl RepoVisibility {
    /// The `gh` flag. One of exactly two literals, whichever variant this is.
    #[must_use]
    pub fn flag(self) -> &'static str {
        match self {
            RepoVisibility::Public => "--public",
            RepoVisibility::Private => "--private",
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RepoVisibility::Public => "public",
            RepoVisibility::Private => "private",
        }
    }
}

/// Every `gh` invocation this crate may make.
///
/// One variant, on purpose. ADR-0008 §2 gives `gh` the whole remote flow
/// precisely so no credential reaches this crate, and one command does all of
/// it. Adding an invocation means adding a variant here, in a diff someone
/// reviews.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GhOp {
    /// `gh repo create --source=. --push --private|--public -- <name>`
    ///
    /// Creates the remote, wires `origin`, and pushes, with `gh` owning
    /// authentication throughout (ADR-0008 §2). The owner is whoever `gh` is
    /// authenticated as — `gh`'s own resolution of its own credential, not
    /// something this crate guesses.
    RepoCreate {
        cwd: PathBuf,
        name: String,
        visibility: RepoVisibility,
    },
}

impl GhOp {
    /// The directory `gh` runs in.
    ///
    /// Load-bearing rather than incidental: `--source=.` names *this*
    /// directory, which is how the source repository is expressed without a
    /// second user-chosen string in argv.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        match self {
            GhOp::RepoCreate { cwd, .. } => cwd,
        }
    }

    /// The `(program, subcommand)` pair the allowlist keys on.
    #[must_use]
    pub fn pair(&self) -> [&'static str; 2] {
        match self {
            GhOp::RepoCreate { .. } => ["repo", "create"],
        }
    }

    /// The argument vector, **constructed** — never assembled from a caller.
    ///
    /// Every element except the trailing name is a literal in this function.
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        let s = |x: &str| x.to_owned();
        match self {
            GhOp::RepoCreate {
                name, visibility, ..
            } => vec![
                s("repo"),
                s("create"),
                // A literal. The directory is the process's cwd, so the source
                // repository is named without putting a path in argv at all.
                s("--source=."),
                s("--push"),
                s(visibility.flag()),
                // End of flags. The name is the one value from outside.
                s("--"),
                name.clone(),
            ],
        }
    }

    /// Whether this op reaches the network. Every variant does, which is why
    /// it selects the long timeout: a create-and-push is seconds to minutes,
    /// where every local `git` call in this crate is budgeted in milliseconds.
    #[must_use]
    pub fn needs_network(&self) -> bool {
        match self {
            GhOp::RepoCreate { .. } => true,
        }
    }

    /// Whether this op can consume a `GITHUB_TOKEN`.
    ///
    /// **Always `false`, and this is the variant ADR-0005 §10 rule 3a expected
    /// to be the exception.** It named `GhOp::RepoCreate` and `GitOp::Push` as
    /// the two ops that would carry a token. ADR-0008 §5 answers the question by
    /// removing it: `gh` owns its own credential, found through the same `HOME`
    /// it needs to find any of its configuration, so handing it a token from our
    /// environment adds a credential channel and buys nothing.
    ///
    /// The cost is stated rather than hidden: a machine whose *only* `gh`
    /// authentication is `GH_TOKEN` in the shell environment will find `gh`
    /// unauthenticated here, because the environment is constructed and that
    /// variable is not forwarded. That is reported as
    /// [`crate::repo::RemoteBlocked::NotAuthenticated`] with the command that
    /// fixes it, which is the honest-detection rule applied to our own
    /// containment.
    #[must_use]
    pub fn needs_credential(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create(name: &str, visibility: RepoVisibility) -> GhOp {
        GhOp::RepoCreate {
            cwd: PathBuf::from("/d/proj"),
            name: name.to_owned(),
            visibility,
        }
    }

    /// Every variant, so the loops below are over the whole enum rather than
    /// over the one someone remembered.
    fn every_variant() -> Vec<GhOp> {
        vec![
            create("demo", RepoVisibility::Private),
            create("demo", RepoVisibility::Public),
        ]
    }

    #[test]
    fn repo_create_argv_is_the_documented_shape() {
        assert_eq!(
            create("demo", RepoVisibility::Private).argv(),
            vec![
                "repo",
                "create",
                "--source=.",
                "--push",
                "--private",
                "--",
                "demo"
            ]
        );
        assert_eq!(create("demo", RepoVisibility::Public).argv()[4], "--public");
    }

    /// **The property this module exists for.** `gh alias set` and
    /// `gh extension install` execute arbitrary binaries by design, and no
    /// variant can express either — not because they are filtered, but because
    /// there is nowhere to put them.
    #[test]
    fn no_variant_can_express_alias_set_or_extension_install() {
        for op in every_variant() {
            assert_eq!(op.pair(), ["repo", "create"], "{op:?}");
            let argv = op.argv();
            for forbidden in ["alias", "extension"] {
                assert_ne!(argv[0], forbidden, "{op:?}");
            }
            let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
            crate::exec::assert_gh_argv_is_clean(&refs)
                .unwrap_or_else(|e| panic!("{op:?} produced {argv:?}: {e:?}"));
        }
    }

    /// The negative control for the test above, and the half that makes it
    /// non-vacuous: the checker must *refuse* the two invocations the enum
    /// cannot express. Without this, a checker that returned `Ok(())`
    /// unconditionally would pass every assertion above.
    #[test]
    fn the_checker_refuses_the_two_invocations_the_enum_cannot_express() {
        for hostile in [
            vec!["alias", "set", "repo", "!sh -c evil"],
            vec!["extension", "install", "owner/evil"],
            // The same idea one level in: our pair, with the dangerous
            // subcommand smuggled into a flag slot.
            vec!["repo", "create", "alias", "--", "demo"],
        ] {
            assert!(
                crate::exec::assert_gh_argv_is_clean(&hostile).is_err(),
                "the gh allowlist admitted {hostile:?}"
            );
        }
    }

    /// The flag list is an allowlist, so a flag nobody vetted is refused even
    /// though it is not on any denylist. `--template` is the worked example:
    /// harmless-sounding, real, and not something any variant here emits.
    #[test]
    fn an_unlisted_flag_is_refused_even_though_no_denylist_names_it() {
        for hostile in [
            vec!["repo", "create", "--template=owner/evil", "--", "demo"],
            vec!["repo", "create", "--source=/etc", "--", "demo"],
            vec!["repo", "create", "--push", "--clone", "--", "demo"],
        ] {
            assert!(
                crate::exec::assert_gh_argv_is_clean(&hostile).is_err(),
                "an unlisted flag was admitted: {hostile:?}"
            );
        }
    }

    /// A project legitimately named `-weird` or `alias` is not an attack, and
    /// refusing it would confuse a containment rule for a validation rule —
    /// the distinction [`crate::git`]'s
    /// `a_flag_shaped_value_is_carried_as_a_value_not_dropped` draws, applied
    /// to the slot where it actually costs something.
    #[test]
    fn a_name_that_reads_like_a_flag_or_a_subcommand_travels_as_a_value() {
        for name in ["-weird", "alias", "extension", "--push"] {
            let argv = create(name, RepoVisibility::Private).argv();
            let sep = argv.iter().position(|a| a == "--").expect("separator");
            assert_eq!(argv.last().map(String::as_str), Some(name));
            assert!(sep < argv.len() - 1, "the name must follow the separator");
            let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
            crate::exec::assert_gh_argv_is_clean(&refs)
                .unwrap_or_else(|e| panic!("`{name}` was refused as a name: {e:?}"));
        }
    }

    #[test]
    fn visibility_is_two_literals_and_there_is_no_third() {
        assert_eq!(RepoVisibility::Public.flag(), "--public");
        assert_eq!(RepoVisibility::Private.flag(), "--private");
        // Whatever the variant, the flag is one of exactly two strings — there
        // is no path from a user string to a `--<something>`.
        for v in [RepoVisibility::Public, RepoVisibility::Private] {
            assert!(["--public", "--private"].contains(&v.flag()));
        }
    }

    #[test]
    fn no_gh_op_carries_a_credential() {
        // ADR-0005 §10 rule 3a named `GhOp::RepoCreate` as the op that would
        // consume a token. If a future variant returns `true`, this fails and
        // somebody has to decide deliberately rather than by omission.
        for op in every_variant() {
            assert!(!op.needs_credential(), "{op:?}");
            assert!(op.needs_network(), "{op:?}");
        }
    }
}
