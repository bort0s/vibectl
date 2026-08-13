//! Whether git's exclusion rules apply to a path — asked, never inferred.
//!
//! This is the instrument ADR-0010 rests on. Under polarity B a prompt is
//! private by default and published by being placed under `shared/`, and the
//! safety property does **not** come from the `.gitignore` lines: it comes from
//! the per-file state being visible without being asked for. An instrument that
//! can quietly report the wrong state turns that display into reassurance.
//!
//! # Three channels, four outcomes, and one of them disappears if you are not
//! careful
//!
//! The outcomes do not arrive through one channel. Three come from an exit
//! status — `0`, `1`, and everything else — and the fourth, `git` being absent
//! altogether, arrives in Rust as a **spawn error**: an `Err` raised before any
//! status exists. There is a third source underneath both, because
//! [`CommandOutput::status`] is `Option<i32>`: `Ok` with no exit code at all.
//!
//! Merging those into one enum is ordinary-looking code, and it is exactly
//! where a stray `unwrap_or`, `.ok()` or `?` collapses absence into the `1`
//! arm — *not ignored*, rendered **not ignored**, which reads as "you're
//! versioned, all good" at the moment the tool has learned nothing. That is the
//! inventing direction, and it is the one that reads as reassurance.
//!
//! Two things in the shape of this module block it structurally rather than by
//! care:
//!
//! 1. **[`check_ignore`] returns [`IgnoreState`], not a `Result`.** There is no
//!    `Result` for a `?` to collapse, and no error type for a caller to discard
//!    with `.ok()`.
//! 2. **[`classify`] matches `status` for `None` before it tests success.**
//!    `CommandOutput::success()` is `false` for a no-exit-code outcome, so the
//!    natural `if success() { … } else { … }` renders it as *not ignored* — the
//!    inventing direction reached by writing the obvious branch, with nobody
//!    making a mistake.
//!
//! # What the labels claim, which is less than they could
//!
//! `ignored` and `not ignored`. Not `tracked`, and not `published`. Exit `1`
//! means **no ignore rule applies**, which is not the same as git having the
//! file: a prompt created ten seconds ago and never `git add`ed is *not
//! ignored* and *not tracked* simultaneously. `check-ignore` answers a question
//! about exclusion rules; the index is a different question and this module does
//! not ask it.
//!
//! What `not ignored` does support is the one inference the feature exists for:
//! **a `git add -A` would pick this file up.**
//!
//! # Versions
//!
//! Every behavioural claim below was measured on **git 2.54.0.windows.1** with
//! `LC_ALL=C`, and the version is part of the claim (ADR-0002 §7). The
//! stderr-derived causes in particular are prose matching against another
//! tool's output; see [`UnknownCause`] for what a wording change costs and what
//! it cannot cost.

use std::path::Path;

use serde::Serialize;

use crate::detect::DetectError;
use crate::error::ErrorPayload;
use crate::exec::{CommandOutput, ProcessRunner};
use crate::git::GitOp;

/// What git's exclusion rules say about one path.
///
/// **Three states, and the third is not decoration.** `Unknown` renders as
/// neither ignored nor not-ignored, in every one of its conditions, because
/// none of them supports a claim about the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum IgnoreState {
    /// An exclusion rule applies. `git check-ignore` exited `0`.
    Ignored,
    /// No exclusion rule applies. `git check-ignore` exited `1`.
    ///
    /// **Not "tracked" and not "published"** — see the module docs. It means a
    /// `git add -A` would pick this file up.
    NotIgnored,
    /// Vibe could not find out.
    ///
    /// The cause is **inside the variant rather than beside it**, so an
    /// `Unknown` with no cause is unrepresentable. Collapsing the sources into
    /// one state is right for the display; dropping the cause is wrong for the
    /// user, because *"git is not installed"* and *"this is not a repository"*
    /// have opposite remedies and an unknown with no cause is honest and
    /// unactionable (ADR-0010 §5).
    Unknown { cause: UnknownCause },
}

impl IgnoreState {
    /// A stable identifier for the *state*, safe to branch on and to print as
    /// data.
    ///
    /// Three keys for three states. The cause is deliberately not folded in
    /// here: a consumer that branches on the state must see one `unknown`,
    /// however many ways there are to reach it.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            IgnoreState::Ignored => "ignored",
            IgnoreState::NotIgnored => "not_ignored",
            IgnoreState::Unknown { .. } => "unknown",
        }
    }

    /// The cause, when the state is `Unknown`.
    #[must_use]
    pub fn cause(&self) -> Option<&UnknownCause> {
        match self {
            IgnoreState::Unknown { cause } => Some(cause),
            IgnoreState::Ignored | IgnoreState::NotIgnored => None,
        }
    }

    /// Whether vibe learned anything at all about this path.
    ///
    /// Exists so a caller writes `if state.is_known()` instead of
    /// `if state == IgnoreState::Ignored` — the second reads a two-way question
    /// out of a three-way answer, which is the collapse this module is built
    /// against.
    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, IgnoreState::Unknown { .. })
    }
}

/// Why vibe could not say whether a path is ignored.
///
/// **A taxonomy, not sentences** (ADR-0001 §4): each variant has a stable
/// [`key`](UnknownCause::key) and carries the variable facts as data, and each
/// frontend writes its own prose. The set is closed and the split is by
/// *remedy*, which is the only thing that makes two causes worth separating —
/// `git` absent and "this is not a repository" are both "vibe cannot say", and
/// a user can act on exactly one of them at a time.
///
/// # The two `128` causes are separated because git's stderr separates them
///
/// ADR-0010 §5 recorded *"not a repo / path outside it"* as one merged cause.
/// Measured on git 2.54.0.windows.1 under `LC_ALL=C`, git distinguishes them,
/// and it does so in prose rather than in the status — all three of these are
/// exit `128`:
///
/// ```text
/// fatal: not a git repository (or any of the parent directories): .git
/// fatal: <p>: '<p>' is outside repository at '<root>'
/// fatal: no path specified
/// ```
///
/// The remedies differ — *initialise or move* versus *the path is wrong* — so
/// they are separate variants. The third has no variant: this crate's closed
/// enum always supplies a path, so a cause we cannot produce would be an arm
/// kept warm for nobody.
///
/// # What a git wording change costs, and what it cannot cost
///
/// Matching another tool's prose is a version-dependent instrument, and this
/// one is pinned to a version rather than assumed stable. But the failure is
/// bounded by construction: an unrecognised message lands in
/// [`UnknownCause::Unrecognised`], which is **still `unknown`**. A wording
/// change degrades how specific the diagnostic is; it can never move the state
/// to `ignored` or `not ignored`, because those two arms are reached only by
/// exit `0` and exit `1` and never by reading stderr at all.
///
/// Staleness gets a trigger rather than a hope: `tests/ignore_state_git.rs`
/// asserts these causes against the real `git` on the machine, and CI runs it
/// under `VIBE_REQUIRE_GIT=1`. A git release that rewords either message turns
/// that test red instead of quietly widening `Unrecognised`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UnknownCause {
    /// `git` could not be run at all. **Remedy: install git.**
    ///
    /// The Rust-side observable is an `Err` from the spawn, never a status —
    /// absence is not in the exit-code space, so through a shell it measures as
    /// `127` and in this crate it does not measure as a number at all.
    ///
    /// **`why` carries what the taxonomy merges.** Two things produce a
    /// [`DetectError::NotAttempted`] here: the spawn failing because `git` is
    /// not on `PATH`, which is the measured and reachable cause, and this
    /// crate's own argument refusal, which
    /// [`GitOp::CheckIgnore`](crate::GitOp::CheckIgnore) cannot trigger for any
    /// path containing a separator. They are not distinguishable from the error
    /// *type*, so they are not claimed to be: one variant, and the difference
    /// travels in `why` rather than in a split the instrument cannot make.
    GitNotRun { why: String },

    /// There is no git repository here. **Remedy: `git init`, or move the
    /// project into a repository.**
    ///
    /// Exit `128` with `fatal: not a git repository…`. Also covers a `.git`
    /// file pointing at a directory that is not there, which reports
    /// `fatal: not a git repository: (NULL)` — from the caller's side that is
    /// the same fact and the same remedy.
    NotARepository,

    /// The path is outside the repository git found. **Remedy: the path is
    /// wrong, or the project root and the path disagree.**
    ///
    /// Exit `128` with `'<p>' is outside repository at '<root>'`.
    ///
    /// **Reachable without leaving the repository**, which is the trap worth
    /// knowing: git rejects a pathspec with a leading `..` on its text, not on
    /// where it resolves. Measured — `../repo/plain.md`, evaluated from inside
    /// `repo`, is reported outside the repository even though it names a file
    /// within it. Pass an absolute path or one relative to the project root.
    PathOutsideRepository,

    /// The query did not finish inside its budget.
    ///
    /// Its own variant rather than folded into [`UnknownCause::GitNotRun`]:
    /// `git` exists and ran, and the remedy — a wedged repository, or a budget
    /// too small — is not "install git".
    TimedOut,

    /// `git` ran and produced **no exit code**.
    ///
    /// **Named for its consequence, not its cause.** On Unix the cause is
    /// termination by signal; on Windows there are no signals and a code is
    /// returned for essentially everything, so `None` there is the
    /// *we-do-not-understand-this* class — which has an even stronger claim to
    /// `unknown`. The cause varies by platform and the mapping does not
    /// (ADR-0010 §10).
    ///
    /// This is the arm the obvious branch gets wrong: `success()` is `false`
    /// here, so `if success() { Ignored } else { NotIgnored }` renders a
    /// process that died as a clean *not ignored*.
    NoExitCode,

    /// `git` ran, exited non-zero, and said something this build does not
    /// recognise.
    ///
    /// The residual arm, and the reason the stderr matching above is safe:
    /// anything unaccounted for lands here as `unknown` with git's own words
    /// attached, rather than being forced into a neighbouring cause or — far
    /// worse — into a state.
    ///
    /// Covers exit codes beyond `128` too: `git check-ignore` exits `129` on a
    /// usage error, measured, which is why the classifier keys on *"not `0`,
    /// not `1`"* rather than on `128`.
    Unrecognised { status: i32, stderr: String },
}

impl UnknownCause {
    /// Every key this enum can produce, for a frontend to check it has a
    /// sentence for each.
    ///
    /// **Keys rather than values**, because two variants carry data and a
    /// `const` array of them is not constructible. The same caveat
    /// [`RemoteBlocked::ALL`](crate::RemoteBlocked::ALL) records applies
    /// unchanged: what this really guarantees is that adding a variant makes
    /// you edit [`UnknownCause::key`]'s exhaustive match (`E0004`) and this
    /// module's `every_cause_has_a_key` test, one of which sits ten lines from
    /// this list. It is not a checked enumeration, and any check driven by
    /// iterating it would be circular.
    pub const ALL_KEYS: [&'static str; 6] = [
        "git_not_run",
        "not_a_repository",
        "path_outside_repository",
        "timed_out",
        "no_exit_code",
        "unrecognised",
    ];

    /// The `(key, code)` pair, from one exhaustive match.
    ///
    /// **Exhaustive on purpose.** `#[non_exhaustive]` does not constrain the
    /// crate that owns the type, so this match is the compile-time break that
    /// makes a new cause impossible to add silently.
    ///
    /// **One match rather than two**, because a variant that answered `key` in
    /// one place and `code` in another could be given a copied code with both
    /// matches still exhaustive and the compiler still satisfied. Here the two
    /// literals sit on one line, and
    /// [`the_code_is_the_key_in_the_wire_format`](self) checks their shape
    /// agrees — the pair cannot be computed at const time from a single
    /// literal, so the relationship is asserted rather than derived, and that
    /// is stated instead of being left to look like duplication.
    fn identity(&self) -> (&'static str, &'static str) {
        match self {
            UnknownCause::GitNotRun { .. } => ("git_not_run", "VIBE_S_IGNORE_GIT_NOT_RUN"),
            UnknownCause::NotARepository => ("not_a_repository", "VIBE_S_IGNORE_NOT_A_REPOSITORY"),
            UnknownCause::PathOutsideRepository => (
                "path_outside_repository",
                "VIBE_S_IGNORE_PATH_OUTSIDE_REPOSITORY",
            ),
            UnknownCause::TimedOut => ("timed_out", "VIBE_S_IGNORE_TIMED_OUT"),
            UnknownCause::NoExitCode => ("no_exit_code", "VIBE_S_IGNORE_NO_EXIT_CODE"),
            UnknownCause::Unrecognised { .. } => ("unrecognised", "VIBE_S_IGNORE_UNRECOGNISED"),
        }
    }

    /// A stable identifier, safe to branch on and safe to print as data.
    #[must_use]
    pub fn key(&self) -> &'static str {
        self.identity().0
    }

    /// The wire code, one per cause.
    ///
    /// **One code per cause, exactly as [`CoreError::code`](crate::CoreError)
    /// gives one per variant, so a consumer learns one rule: branch on `code`.**
    /// This replaced a single `VIBE_S_IGNORE_UNKNOWN` shared by all six, under
    /// which `NotARepository`, `PathOutsideRepository`, `TimedOut` and
    /// `NoExitCode` carried no params either and were distinguishable only by
    /// the `message` — so telling *install git* from *`git init`*, the pair §5
    /// separates precisely because their remedies are opposite, meant parsing
    /// prose.
    ///
    /// The rejected alternative was a mandatory `params["cause"]`: `params` is a
    /// `BTreeMap`, so "mandatory" would be a convention enforced by a test where
    /// `code` is a field enforced by the type — a contract asserted in prose
    /// about a container that cannot carry it, which is the defect this change
    /// removes, rebuilt one slot over.
    ///
    /// **What the split costs.** `VIBE_S_IGNORE_UNKNOWN` was also a single
    /// handle for *the class*, and six codes are not. That is affordable because
    /// this type exists only inside [`IgnoreState::Unknown`], so the class is
    /// always in hand where a payload is built and already travels as
    /// `state: "unknown"` on [`IgnoreState`]'s own `Serialize`. The payload
    /// carries the diagnostic; the state carries the class. A consumer logging
    /// payloads stripped of their states is the one case that loses it.
    ///
    /// The `VIBE_S_` prefix survives and keeps its one job: an unknown state is
    /// **not an error** (`VIBE_E_`), it is a measurement that did not land.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.identity().1
    }

    /// The `{ code, params }` projection (ADR-0005 §6).
    ///
    /// The state is one and the diagnostic is specific, and this is the shape
    /// that carries the specific half. Each slot carries a different thing and
    /// none of them substitutes for another: **`code` identifies, `message`
    /// describes, `params` carries the variable data — and the *remedy* is the
    /// frontend's** (ADR-0001 §4).
    ///
    /// # `message` is a description, and that is a contract
    ///
    /// This doc once said the `message` was *"a fallback rather than a
    /// contract"*. That was false when it was written: `vibectl` renders
    /// `ErrorPayload::message` verbatim to users, so the only frontend that
    /// exists already relies on it. Calling a relied-on field a fallback does
    /// not make it one; it just leaves the reliance unrecorded.
    ///
    /// The honest split, which is also what `error_lines` has always done —
    /// `error: {description}` then `hint: {remedy}`:
    ///
    /// - **Core owns the description.** For this type it is the *only* English
    ///   that exists: `UnknownCause` has no `Display`, so a frontend with no
    ///   description of its own has nothing else to print.
    /// - **The frontend owns the remedy**, keyed on [`UnknownCause::code`].
    ///   ADR-0010 §5 separates these causes *by remedy* in the first place, so
    ///   the per-cause catalogue a frontend writes is a remedy catalogue and is
    ///   not a second description of the same six things.
    ///
    /// What stopped being supported is **branching on `message`**, which the
    /// one-code-per-cause split above is what makes unnecessary.
    ///
    /// [`ErrorPayload`] is reused rather than duplicated even though an unknown
    /// state is **not an error** — it is a measurement that did not land. The
    /// wire shape is the same shape, and inventing a second one would give
    /// frontends two catalogs to key off instead of one.
    #[must_use]
    pub fn to_wire(&self) -> ErrorPayload {
        let mut params = std::collections::BTreeMap::new();
        let message = match self {
            UnknownCause::GitNotRun { why } => {
                params.insert("why".to_owned(), why.clone());
                format!("git could not be run: {why}")
            }
            UnknownCause::NotARepository => "not a git repository".to_owned(),
            UnknownCause::PathOutsideRepository => "path is outside the repository".to_owned(),
            UnknownCause::TimedOut => "git check-ignore did not finish in time".to_owned(),
            UnknownCause::NoExitCode => "git produced no exit code".to_owned(),
            // Both halves travel: `status` is the structured discriminant a
            // consumer branches on, `stderr` is git's own prose carried
            // alongside and never instead.
            UnknownCause::Unrecognised { status, stderr } => {
                params.insert("status".to_owned(), status.to_string());
                params.insert("stderr".to_owned(), stderr.clone());
                format!("git check-ignore exited {status}")
            }
        };

        ErrorPayload {
            code: self.code(),
            message,
            path: None,
            path_lossy: false,
            params,
            chain: Vec::new(),
        }
    }
}

/// The markers this build recognises in `git check-ignore`'s stderr.
///
/// Measured on git 2.54.0.windows.1 with `LC_ALL=C`, which
/// [`child_env`](crate::exec) sets positively — so the prose is not at the
/// mercy of the user's locale even though it is at the mercy of git's version.
mod marker {
    /// From `fatal: <p>: '<p>' is outside repository at '<root>'`.
    ///
    /// Includes the trailing quote and `at` so the marker cannot be produced by
    /// a file whose *name* contains the words.
    pub(super) const OUTSIDE_REPOSITORY: &str = "is outside repository at '";

    /// From `fatal: not a git repository (or any of the parent directories):
    /// .git` and from `fatal: not a git repository: (NULL)`.
    ///
    /// Matched as a **line prefix**, not a substring. git interpolates the
    /// offending path immediately after `fatal: ` in the outside-repository
    /// message, so a substring match here would let a path named
    /// `not a git repository` steer the classification.
    pub(super) const NOT_A_REPOSITORY: &str = "fatal: not a git repository";
}

/// Ask git whether its exclusion rules apply to `path`.
///
/// `project_dir` is the directory git runs in. `path` reaches git verbatim as a
/// pathspec after `--`; an absolute path and one relative to `project_dir` are
/// both measured to work. A path with a leading `..` is reported as
/// [`UnknownCause::PathOutsideRepository`] **even when it resolves back inside**
/// the repository, so callers should not build one.
///
/// # Returns a state, never a `Result`
///
/// Every way this can fail is a *state* — including `git` being absent, which is
/// a fact about the machine and not about the file. There is no error to
/// discard, which is the point: see the module docs for the collapse this
/// shape exists to prevent.
///
/// # Why `git_available()` is not consulted first
///
/// It would be a second subprocess answering a question this one already
/// answers, and the two could disagree — the harness-versus-subject mistake
/// (ADR-0002 §7) built into shipping code, where nothing would catch it. The
/// absence is read from the invocation that matters.
#[must_use]
pub fn check_ignore(project_dir: &Path, path: &Path, exec: &dyn ProcessRunner) -> IgnoreState {
    classify(exec.run_git_op(&GitOp::CheckIgnore {
        cwd: project_dir.to_path_buf(),
        path: path.to_path_buf(),
    }))
}

/// Map one raw invocation outcome onto a state.
///
/// **Factored out and public so it can be tested over values the machine
/// running the tests cannot produce** — a no-exit-code outcome is unreachable on
/// a Windows runner, and this is what lets the mapping be asserted on all three
/// platforms anyway (ADR-0010 §10). The reachable half is not given up in
/// exchange: `tests/ignore_state_git.rs` drives the same function from a real
/// signal-killed process, gated to Unix.
#[must_use]
pub fn classify(outcome: Result<CommandOutput, DetectError>) -> IgnoreState {
    let unknown = |cause| IgnoreState::Unknown { cause };

    // Exhaustive over `DetectError`, with no wildcard: `#[non_exhaustive]` does
    // not constrain this crate, so a new failure mode must be routed here
    // deliberately rather than falling into a default that means "git is
    // missing".
    let out = match outcome {
        Ok(out) => out,
        Err(DetectError::Timeout) => return unknown(UnknownCause::TimedOut),
        Err(err @ (DetectError::NotAttempted { .. } | DetectError::Unreadable { .. })) => {
            return unknown(UnknownCause::GitNotRun {
                why: err.to_string(),
            });
        }
    };

    // **Before any success test.** `out.success()` is `false` here and `false`
    // is also what exit `1` produces, so a branch on success cannot tell a
    // process that died from a file that is simply not ignored.
    let Some(status) = out.status else {
        return unknown(UnknownCause::NoExitCode);
    };

    match status {
        0 => IgnoreState::Ignored,
        1 => IgnoreState::NotIgnored,
        // Not `128 => …` with a fallthrough: `git check-ignore` exits `129` on
        // a usage error, so keying on the one number we happened to measure
        // would leave a second failure code with nowhere to go.
        other => unknown(classify_failure(other, &out.stderr)),
    }
}

/// Read the cause out of git's own words.
///
/// Order matters and is not alphabetical. The outside-repository marker is
/// tested first because it is the harder of the two to forge — it needs the
/// quote-and-`at` suffix — and because the not-a-repository message never
/// interpolates a path, so it can never contain the other marker. Reversing
/// these would let a file under a directory named `not a git repository` be
/// classified by its own name.
fn classify_failure(status: i32, stderr: &str) -> UnknownCause {
    if stderr.contains(marker::OUTSIDE_REPOSITORY) {
        return UnknownCause::PathOutsideRepository;
    }
    if stderr
        .lines()
        .any(|line| line.starts_with(marker::NOT_A_REPOSITORY))
    {
        return UnknownCause::NotARepository;
    }
    UnknownCause::Unrecognised {
        status,
        stderr: stderr.trim().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(status: Option<i32>, stderr: &str) -> CommandOutput {
        CommandOutput {
            argv: vec!["git".to_owned(), "check-ignore".to_owned()],
            status,
            stdout: String::new(),
            stderr: stderr.to_owned(),
        }
    }

    // --- the two arms that are allowed to claim something ------------------

    #[test]
    fn exit_zero_is_ignored_and_exit_one_is_not_ignored() {
        assert_eq!(classify(Ok(output(Some(0), ""))), IgnoreState::Ignored);
        assert_eq!(classify(Ok(output(Some(1), ""))), IgnoreState::NotIgnored);
    }

    /// The paired half of every "this is unknown" assertion below.
    ///
    /// An implementation that returned `Unknown` unconditionally — a broken
    /// invocation, a swallowed error, a state machine wired to one arm —
    /// satisfies every unknown assertion perfectly, and the green would mean
    /// nothing (ADR-0010 §10).
    #[test]
    fn the_known_arms_are_reachable_so_the_unknown_assertions_are_not_vacuous() {
        assert!(classify(Ok(output(Some(0), ""))).is_known());
        assert!(classify(Ok(output(Some(1), ""))).is_known());
        assert_eq!(classify(Ok(output(Some(0), ""))).cause(), None);
    }

    // --- the arms that must never claim anything ---------------------------

    /// The synthesised no-exit-code value ADR-0010 §10 asks for, so the mapping
    /// is covered on all three platforms including the one where the outcome is
    /// unreachable. Its reachable counterpart is the Unix-gated `SIGKILL` test
    /// in `tests/ignore_state_git.rs`.
    #[test]
    fn no_exit_code_is_unknown_and_emphatically_not_not_ignored() {
        let state = classify(Ok(output(None, "")));
        assert_eq!(
            state,
            IgnoreState::Unknown {
                cause: UnknownCause::NoExitCode
            }
        );
        // The whole point, spelled out: these two are one boolean apart and
        // only one of them is a lie.
        assert_ne!(state, IgnoreState::NotIgnored);
    }

    #[test]
    fn a_spawn_failure_is_git_not_run_and_carries_why() {
        let state = classify(Err(DetectError::NotAttempted {
            why: "could not run git: program not found".to_owned(),
        }));
        let Some(UnknownCause::GitNotRun { why }) = state.cause() else {
            panic!("expected git_not_run, got {state:?}");
        };
        assert!(why.contains("program not found"), "{why}");
        assert_ne!(state, IgnoreState::NotIgnored);
    }

    #[test]
    fn a_timeout_is_its_own_cause_not_a_missing_git() {
        assert_eq!(
            classify(Err(DetectError::Timeout)).cause(),
            Some(&UnknownCause::TimedOut)
        );
    }

    // --- the 128 split, which is the thing this diff decided ---------------

    #[test]
    fn the_two_128_causes_are_told_apart_by_stderr() {
        let not_a_repo = classify(Ok(output(
            Some(128),
            "fatal: not a git repository (or any of the parent directories): .git\n",
        )));
        assert_eq!(not_a_repo.cause(), Some(&UnknownCause::NotARepository));

        let outside = classify(Ok(output(
            Some(128),
            "fatal: ../x/a.md: '../x/a.md' is outside repository at '/d/repo'\n",
        )));
        assert_eq!(outside.cause(), Some(&UnknownCause::PathOutsideRepository));

        // Different causes, same state. Both remain `unknown`, because neither
        // supports a claim about the file.
        assert_eq!(not_a_repo.key(), "unknown");
        assert_eq!(outside.key(), "unknown");
    }

    /// A `.git` file pointing at a directory that is not there. Measured
    /// wording, and it is not the wording of the common case.
    #[test]
    fn a_broken_gitdir_pointer_is_also_not_a_repository() {
        assert_eq!(
            classify(Ok(output(
                Some(128),
                "fatal: not a git repository: (NULL)\n"
            )))
            .cause(),
            Some(&UnknownCause::NotARepository)
        );
    }

    /// The ordering that keeps a *file name* from steering the classification.
    #[test]
    fn a_path_named_like_the_other_marker_does_not_steer_the_cause() {
        // A real outside-repository failure for a path that happens to contain
        // the not-a-repository wording. Substring-matching in the wrong order
        // would call this `not_a_repository` and send the user to `git init`.
        let stderr = "fatal: not a git repository/x.md: 'not a git repository/x.md' \
                      is outside repository at '/d/repo'\n";
        assert_eq!(
            classify(Ok(output(Some(128), stderr))).cause(),
            Some(&UnknownCause::PathOutsideRepository)
        );
    }

    // --- the residual arm, which is what makes the prose matching safe -----

    #[test]
    fn an_unrecognised_failure_stays_unknown_and_keeps_gits_words() {
        // What a future git release's reworded message looks like from here.
        let state = classify(Ok(output(Some(128), "fatal: something new in 2.99\n")));
        let Some(UnknownCause::Unrecognised { status, stderr }) = state.cause() else {
            panic!("expected unrecognised, got {state:?}");
        };
        assert_eq!(*status, 128);
        assert_eq!(stderr, "fatal: something new in 2.99");
        // The bound on prose matching: specificity degrades, the state cannot.
        assert!(!state.is_known());
        assert_ne!(state, IgnoreState::NotIgnored);
    }

    /// `129`, measured from `git check-ignore --<unknown-flag>`. A classifier
    /// keyed on `128` would have nowhere to put this.
    #[test]
    fn a_usage_error_is_unknown_rather_than_not_ignored() {
        let state = classify(Ok(output(Some(129), "error: unknown option `x'\n")));
        assert_eq!(state.key(), "unknown");
        assert!(matches!(
            state.cause(),
            Some(UnknownCause::Unrecognised { status: 129, .. })
        ));
    }

    /// An empty stderr with a failing status must not fall through to a state.
    /// This is the shape a swallowed pipe would produce.
    #[test]
    fn a_failure_with_no_stderr_at_all_is_still_unknown() {
        assert!(!classify(Ok(output(Some(128), ""))).is_known());
    }

    // --- taxonomy hygiene --------------------------------------------------

    /// One value per variant, for the checks that must visit all of them.
    ///
    /// A hand-written list, and therefore the same bounded thing
    /// [`UnknownCause::ALL_KEYS`] is: what makes it hard to leave stale is that
    /// `every_cause_has_a_key_and_the_keys_are_distinct` compares its length
    /// against `ALL_KEYS`, and `ALL_KEYS` sits beside an exhaustive match that
    /// does not compile when a variant is added.
    fn all_causes() -> [UnknownCause; 6] {
        [
            UnknownCause::GitNotRun { why: "w".into() },
            UnknownCause::NotARepository,
            UnknownCause::PathOutsideRepository,
            UnknownCause::TimedOut,
            UnknownCause::NoExitCode,
            UnknownCause::Unrecognised {
                status: 128,
                stderr: "s".into(),
            },
        ]
    }

    #[test]
    fn every_cause_has_a_key_and_the_keys_are_distinct() {
        let all = all_causes();
        let mut keys: Vec<&str> = all.iter().map(UnknownCause::key).collect();
        assert_eq!(
            keys.len(),
            UnknownCause::ALL_KEYS.len(),
            "ALL_KEYS is stale"
        );
        for key in &keys {
            assert!(
                UnknownCause::ALL_KEYS.contains(key),
                "{key} is missing from ALL_KEYS"
            );
        }
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "duplicate cause keys: {keys:?}");
    }

    #[test]
    fn the_state_keys_are_three_and_distinct() {
        assert_eq!(IgnoreState::Ignored.key(), "ignored");
        assert_eq!(IgnoreState::NotIgnored.key(), "not_ignored");
        assert_eq!(
            IgnoreState::Unknown {
                cause: UnknownCause::NoExitCode
            }
            .key(),
            "unknown"
        );
    }

    /// **The thing the one-code-per-cause split exists for.**
    ///
    /// Before it, these four carried the same `code` *and* no params, so the
    /// only way to tell them apart on the wire was the `message` — prose. Two of
    /// them are the pair ADR-0010 §5 separates precisely because the remedies
    /// are opposite: *install git* versus *`git init`*.
    #[test]
    fn the_four_paramless_causes_are_told_apart_by_their_codes() {
        let bare = [
            UnknownCause::NotARepository,
            UnknownCause::PathOutsideRepository,
            UnknownCause::TimedOut,
            UnknownCause::NoExitCode,
        ];
        let wires: Vec<ErrorPayload> = bare.iter().map(UnknownCause::to_wire).collect();

        // The premise, asserted rather than assumed: these really do carry no
        // params, so `code` is the *whole* discriminant and not a convenience
        // beside one. Without this the test below would pass on a build where
        // params had quietly become the discriminant instead.
        for wire in &wires {
            assert!(wire.params.is_empty(), "{} gained params", wire.code);
        }

        let mut codes: Vec<&str> = wires.iter().map(|w| w.code).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(before, codes.len(), "two causes share a code: {codes:?}");
    }

    #[test]
    fn every_cause_has_its_own_code() {
        let all = all_causes();
        let mut codes: Vec<&str> = all.iter().map(UnknownCause::code).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(before, codes.len(), "duplicate cause codes: {codes:?}");
    }

    /// The two literals in `identity` cannot be computed from each other at
    /// const time, so their agreement is checked instead of derived.
    ///
    /// This is what catches the copy-paste a single exhaustive match otherwise
    /// permits: a new variant given a neighbour's code compiles perfectly.
    #[test]
    fn the_code_is_the_key_in_the_wire_format() {
        for cause in all_causes() {
            assert_eq!(
                cause.code(),
                format!("VIBE_S_IGNORE_{}", cause.key().to_ascii_uppercase()),
                "`{}` and `{}` do not agree",
                cause.key(),
                cause.code()
            );
        }
    }

    /// **Two discriminants reach the wire and they must not drift.**
    ///
    /// `code` comes from a literal in `identity`; `cause.cause` comes from
    /// serde's `rename_all` over the *variant name*. Renaming a variant moves
    /// the second and leaves the first, and nothing else in this crate would
    /// notice — a frontend keyed on one would then disagree with a frontend
    /// keyed on the other about the same value.
    #[test]
    fn the_serialised_tag_is_the_key() {
        for cause in all_causes() {
            let json = serde_json::to_value(&cause).expect("serialize");
            assert_eq!(
                json["cause"],
                cause.key(),
                "serde tag and key disagree for {cause:?}"
            );
        }
    }

    /// The diagnostic is specific even though the state is one, and the
    /// structured discriminants travel next to the prose rather than inside it
    /// (ADR-0005 §6).
    #[test]
    fn the_cause_projects_to_code_and_params() {
        let wire = UnknownCause::Unrecognised {
            status: 128,
            stderr: "fatal: whatever".to_owned(),
        }
        .to_wire();
        assert_eq!(wire.code, "VIBE_S_IGNORE_UNRECOGNISED");
        assert_eq!(wire.params["status"], "128");
        assert_eq!(wire.params["stderr"], "fatal: whatever");

        let wire = UnknownCause::GitNotRun {
            why: "could not run git: not found".to_owned(),
        }
        .to_wire();
        assert_eq!(wire.params["why"], "could not run git: not found");

        // A cause with nothing variable carries no params, and that is not the
        // same as carrying an empty one: the code is the whole message.
        assert!(UnknownCause::NotARepository.to_wire().params.is_empty());
    }

    /// The states serialize as data a frontend can switch on, with the cause
    /// nested rather than flattened into the state.
    #[test]
    fn the_state_serializes_with_its_cause() {
        let json = serde_json::to_value(IgnoreState::Unknown {
            cause: UnknownCause::NotARepository,
        })
        .unwrap();
        assert_eq!(json["state"], "unknown");
        assert_eq!(json["cause"]["cause"], "not_a_repository");

        let json = serde_json::to_value(IgnoreState::NotIgnored).unwrap();
        assert_eq!(json["state"], "not_ignored");
    }
}
