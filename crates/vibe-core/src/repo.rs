//! Initialising a repository, and saying honestly what was not done.
//!
//! These operations do **not** travel inside a [`crate::WritePlan`], for the
//! reason recorded in ADR-0006 §9b: `git init` writes `<project>/.git/**`, and
//! containment rule 6 rejects any op path with a `.git` component. Routing them
//! through `apply` would buy nothing — the check is on the op's declared path,
//! never on what the subprocess goes on to write — and would cost an exception
//! to a rule whose whole value is not having one.
//!
//! What applies instead is rules 1–4: the closed enum, constructed argv, the
//! constructed environment, and a validated [`crate::GitUrl`] anywhere a URL
//! reaches argv.
//!
//! # The reporting is the feature
//!
//! On the path where `gh` is absent, `vibe` creates no remote and pushes
//! nothing. The quality of that path is entirely the quality of one message, so
//! this module returns **structured facts** and `vibectl` writes the sentence —
//! core never holds prose (ADR-0001 §4).
//!
//! The facts are shaped so a consumer cannot accidentally report a limitation
//! as a property of the project. "`gh` is not installed" is a fact about this
//! machine, exactly like `SyncNotes::not_attempted`, and it must never read as
//! "this project cannot have a remote".

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::CoreError;
use crate::exec::ProcessRunner;
use crate::gh::{GhOp, RepoVisibility};
use crate::git::GitOp;

/// The files a fresh scaffold contains, staged in a fixed order.
///
/// A constructed list rather than `git add -A`. `-A` stages whatever happens to
/// be in the directory, which for `vibe new` is exactly what we wrote — but the
/// day someone runs this against a directory that is not fresh, `-A` commits
/// their untracked work under our message. Naming the paths keeps the operation
/// describing what it does.
const SCAFFOLD_PATHS: &[&str] = &[".vibe"];

/// Why the scaffold was not committed.
///
/// Both variants are **facts about this machine**, not failures of the command,
/// and neither is a reason to fail the whole run. Collapsing them into an error
/// would make `vibe new --git` unusable on a fresh machine, and inventing an
/// author identity to avoid the second would attribute a commit to a person who
/// does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CommitBlocked {
    /// Nothing was staged. A re-run in an already-committed project.
    NothingToCommit,
    /// `git` has no `user.name`/`user.email` on this machine.
    ///
    /// Common on a fresh install, and **not ours to fix**. Setting one would
    /// mean choosing a name and address on the user's behalf and stamping them
    /// into history, which is the same class of invention as writing a
    /// plausible value into a manifest field nothing detected.
    NoAuthorIdentity,
}

impl CommitBlocked {
    /// A stable identifier, safe to branch on and safe to print as data.
    ///
    /// **Not a sentence.** ADR-0001 §4: core carries the taxonomy, each
    /// frontend writes its own prose, and a string that lives in both places
    /// drifts. The `as_str` this replaces held a fragment of English that
    /// `vibectl` duplicated in its own renderer and asserted only its own copy
    /// of.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            CommitBlocked::NothingToCommit => "nothing_to_commit",
            CommitBlocked::NoAuthorIdentity => "no_author_identity",
        }
    }
}

/// Why the remote was not created, when the user asked for one.
///
/// Every variant is a **fact about this machine or this run**, never a property
/// of the project — the `NotAttempted`-versus-`NoEvidence` distinction applied
/// to the tool's own capability (ADR-0008 §3). A consumer that renders any of
/// these as "this project cannot have a remote" has made the mistake in a new
/// place.
///
/// The set is closed and short on purpose. Anything not listed here is a real
/// failure and surfaces as [`CoreError::ToolFailed`] carrying `gh`'s own
/// stderr: a catch-all variant would turn every `gh` problem into a silent
/// "no remote", which is the same swallow [`classify_commit_failure`] refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RemoteBlocked {
    /// `gh` could not be run on this machine.
    GhMissing,
    /// `gh` ran and has no credential it can use.
    ///
    /// Includes the case this crate's own containment creates: `gh` reads
    /// `GH_TOKEN` from the environment, and the environment handed to it is
    /// constructed rather than inherited, so a machine authenticated *only* by
    /// an exported token looks unauthenticated here (ADR-0008 §5). Reported
    /// with the command that fixes it rather than worked around by forwarding
    /// a credential.
    NotAuthenticated,
    /// There is no commit to push, so there is nothing to create a remote for.
    ///
    /// Checked before `gh` runs rather than after it fails: `gh repo create
    /// --push` on a repository with no commits creates the remote and then
    /// fails at the push, which leaves an empty repository on the user's
    /// account as the side effect of a command that reported failure.
    NothingToPush,
}

impl RemoteBlocked {
    /// Every variant, for a frontend to check it has a sentence for each.
    ///
    /// **This list is a second place to remember, and that is stated rather
    /// than papered over.** Rust cannot enumerate an enum's variants without a
    /// macro or a derive, so nothing makes `ALL` complete by construction: add
    /// a variant, forget to list it here, and a frontend's coverage test stays
    /// green over a shorter list. What is available — and what
    /// [`RemoteBlocked::all_is_complete`] does — is to put an **exhaustive
    /// match beside the list**, so a new variant fails to compile in this file,
    /// two lines from the thing that needs updating. The break lands next to
    /// the list; it is not derived from it. See ADR-0001 §4.
    pub const ALL: [Self; 3] = [Self::GhMissing, Self::NotAuthenticated, Self::NothingToPush];

    /// A stable identifier, safe to branch on and safe to print as data.
    ///
    /// **Exhaustive on purpose.** `#[non_exhaustive]` does not constrain the
    /// crate that owns the type, so this match is the compile-time break that
    /// makes a new variant impossible to add silently: core stops building,
    /// the author adds a key, `ALL` grows, and each frontend's coverage test
    /// then goes red until someone writes the sentence (ADR-0001 §4).
    ///
    /// It is a key, not prose. `vibectl` and the desktop app write different
    /// sentences for the same reason, and neither is core's business.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            RemoteBlocked::GhMissing => "gh_missing",
            RemoteBlocked::NotAuthenticated => "not_authenticated",
            RemoteBlocked::NothingToPush => "nothing_to_push",
        }
    }
}

/// What repository setup actually did, and what it did not.
///
/// Every field is a fact, never a sentence. `vibectl` turns these into the
/// finish-the-job message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RepoReport {
    /// A repository was created here by this run.
    pub initialised: bool,
    /// Already a repository before we looked. Not an error — `vibe new --git`
    /// on an existing repository is a reasonable thing to do by accident, and
    /// re-initialising would be a write nobody asked for.
    pub already_a_repository: bool,
    /// The scaffold was staged and committed.
    pub committed: bool,
    /// Why not, when it was not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_blocked: Option<CommitBlocked>,
    /// The branch the repository is actually on.
    ///
    /// Read via `git symbolic-ref`, never assumed. `git init` honours the
    /// user's `init.defaultBranch`, so a hard-coded `main` in the finish-the-job
    /// message would be a plausible-looking guess in the one output whose entire
    /// purpose is being correct enough to paste (ADR-0008 §7).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Whether `gh` could be run at all.
    ///
    /// **A fact about this machine.** It is not a property of the project, and
    /// a consumer that renders it as one has made the `NotAttempted`-versus-
    /// `NoEvidence` mistake in a new place.
    pub gh_available: bool,
    /// The visibility the caller asked the remote to be created with, or
    /// `None` if no remote was asked for.
    ///
    /// One field rather than a `remote_requested: bool` beside a visibility,
    /// because the two can disagree and one of the two disagreements is
    /// "create a repository whose visibility nobody chose".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_requested: Option<RepoVisibility>,
    /// `gh` created the remote and pushed.
    pub remote_created: bool,
    /// Why not, when a remote was asked for and there is not one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_blocked: Option<RemoteBlocked>,
    /// The remote `origin` actually points at, read back after the fact.
    ///
    /// Read with `git remote get-url`, not parsed out of `gh`'s output. `gh`
    /// prints a URL in prose that changes between releases; `git` reports what
    /// is in `.git/config`, which is the thing that is actually true. Same rule
    /// as [`RepoReport::branch`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
}

impl RepoReport {
    /// Whether the user has anything left to do.
    ///
    /// Note what is **not** here: `gh` being absent is only something left to
    /// do if a remote was asked for. `vibe new --git` on its own is a request
    /// for a local repository, and reporting a limitation nobody reached would
    /// be nagging about a problem the user does not have.
    #[must_use]
    pub fn needs_manual_finish(&self) -> bool {
        self.commit_blocked == Some(CommitBlocked::NoAuthorIdentity)
            || (self.remote_requested.is_some() && !self.remote_created)
    }
}

/// Initialise a repository, commit the scaffold, and — only if asked — create
/// the remote.
///
/// Idempotent in the direction that matters: an existing repository is left
/// alone rather than re-initialised, and a scaffold that is already committed
/// produces no second commit.
///
/// `remote` is `Option<RepoVisibility>` rather than a `bool` plus a default,
/// and the shape is the decision. Creating a repository on github.com is
/// outward-facing and not undoable by this tool, so it happens only when the
/// caller names the visibility — there is no value we could pick that is not us
/// deciding whether someone's code is published.
pub fn init(
    project_dir: &Path,
    exec: &dyn ProcessRunner,
    message: &str,
    remote: Option<RepoVisibility>,
) -> Result<RepoReport, CoreError> {
    if !exec.git_available() {
        return Err(CoreError::GitUnavailable {
            why: "git is not on PATH".to_owned(),
        });
    }

    let already = project_dir.join(".git").exists();
    if !already {
        run(
            exec,
            &GitOp::Init {
                cwd: project_dir.to_path_buf(),
            },
        )?;
    }

    let paths: Vec<PathBuf> = SCAFFOLD_PATHS
        .iter()
        .map(PathBuf::from)
        .filter(|p| project_dir.join(p).exists())
        .collect();
    let mut committed = false;
    let mut commit_blocked = None;
    if paths.is_empty() {
        commit_blocked = Some(CommitBlocked::NothingToCommit);
    } else {
        run(
            exec,
            &GitOp::Add {
                cwd: project_dir.to_path_buf(),
                paths,
            },
        )?;
        // A commit with nothing staged exits non-zero and says "nothing to
        // commit". That is a no-op, not a failure — re-running `vibe new --git`
        // in a directory whose scaffold is already committed must not error.
        let out = exec
            .run_git_op(&GitOp::Commit {
                cwd: project_dir.to_path_buf(),
                message: message.to_owned(),
            })
            .map_err(|e| CoreError::GitUnavailable { why: e.to_string() })?;
        committed = out.success();
        if !committed {
            // Two recognised non-failures. Anything else is a real error and
            // must not be swallowed - a catch-all here would turn every commit
            // problem into a silent "not committed".
            commit_blocked = classify_commit_failure(&out);
            if commit_blocked.is_none() {
                return Err(CoreError::ToolFailed {
                    argv: out.argv.clone(),
                    status: out.status,
                    stderr: out.stderr.trim().to_owned(),
                });
            }
        }
    }

    let gh_available = exec.gh_available();
    let remote_outcome = match remote {
        Some(visibility) => create_remote(project_dir, exec, visibility, committed, gh_available)?,
        // Not asked for, so not attempted, so nothing to report about it.
        None => RemoteOutcome::default(),
    };

    Ok(RepoReport {
        initialised: !already,
        already_a_repository: already,
        committed,
        commit_blocked,
        // Read after the commit: before the first commit `HEAD` is unborn, and
        // `symbolic-ref` still resolves it, but reading it afterwards is the
        // state the user will actually be pushing.
        branch: current_branch(project_dir, exec),
        gh_available,
        remote_requested: remote,
        remote_created: remote_outcome.created,
        remote_blocked: remote_outcome.blocked,
        remote_url: remote_outcome.url,
    })
}

/// What the remote step did. Internal, so [`RepoReport`] stays a flat record.
#[derive(Debug, Default)]
struct RemoteOutcome {
    created: bool,
    blocked: Option<RemoteBlocked>,
    url: Option<String>,
}

/// Hand the whole remote flow to `gh`, or say precisely why it did not run.
///
/// One `gh repo create --source=. --push` does the create, the `origin` wiring
/// and the push, with `gh` owning authentication throughout (ADR-0008 §2).
/// That is not an efficiency: it is the reason no credential reaches this
/// crate, because there is no step here that would need one.
///
/// The two pre-flight checks run **before** `gh` does, and both are about not
/// leaving debris on a user's account. A missing `gh` obviously cannot create
/// anything; a repository with no commit gets created and then fails at the
/// push, which is a worse outcome than declining.
///
/// **They are ordered by what the user has to fix first, not by what is
/// cheapest to check.** Both can hold at once — a fresh machine often has
/// neither a git identity nor `gh` — and reporting "gh was not found" to
/// someone whose actual first blocker is an uncommitted scaffold sends them to
/// install a tool that would not have run anyway. The chain is: get a commit,
/// then get `gh`.
fn create_remote(
    project_dir: &Path,
    exec: &dyn ProcessRunner,
    visibility: RepoVisibility,
    committed: bool,
    gh_available: bool,
) -> Result<RemoteOutcome, CoreError> {
    let blocked = |why: RemoteBlocked| {
        Ok(RemoteOutcome {
            created: false,
            blocked: Some(why),
            url: None,
        })
    };

    if !committed {
        return blocked(RemoteBlocked::NothingToPush);
    }
    if !gh_available {
        return blocked(RemoteBlocked::GhMissing);
    }

    // The name `gh` gives the repository is the directory's own name, which is
    // the project name `vibe new` created it from. Deriving it here rather than
    // taking it as an argument keeps the two from drifting: the repository is
    // named after the directory that is being pushed, always.
    let Some(name) = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
    else {
        return Err(CoreError::GitUnavailable {
            why: format!(
                "cannot name a repository after {} — it has no final path component",
                project_dir.display()
            ),
        });
    };

    let out = exec
        .run_gh_op(&GhOp::RepoCreate {
            cwd: project_dir.to_path_buf(),
            name,
            visibility,
        })
        .map_err(|e| CoreError::GitUnavailable { why: e.to_string() })?;

    if out.success() {
        return Ok(RemoteOutcome {
            created: true,
            blocked: None,
            url: remote_url(project_dir, exec),
        });
    }
    match classify_gh_failure(&out) {
        Some(why) => blocked(why),
        // Unrecognised means unrecognised. Turning it into a quiet "no remote"
        // would hide `gh`'s own explanation of what went wrong.
        None => Err(CoreError::ToolFailed {
            argv: out.argv.clone(),
            status: out.status,
            stderr: out.stderr.trim().to_owned(),
        }),
    }
}

/// What `origin` points at, or `None`.
///
/// Reuses the store's `remote get-url origin` op. Its docs describe it as the
/// store's ownership check, which is what it was written for; the invocation is
/// `git remote get-url origin` either way, and reading a fact back off disk is
/// the same reason both callers want it.
fn remote_url(project_dir: &Path, exec: &dyn ProcessRunner) -> Option<String> {
    let out = exec
        .run_git_op(&GitOp::RemoteGetUrl {
            cwd: project_dir.to_path_buf(),
        })
        .ok()?;
    if !out.success() {
        return None;
    }
    let url = out.trimmed().trim().to_owned();
    if url.is_empty() { None } else { Some(url) }
}

/// The branch, or `None` if it could not be read.
///
/// `None` rather than a default. A branch we could not determine is not `main`,
/// and printing `main` because we did not know would be the exact substitution
/// this project forbids everywhere else.
#[must_use]
pub fn current_branch(project_dir: &Path, exec: &dyn ProcessRunner) -> Option<String> {
    let out = exec
        .run_git_op(&GitOp::CurrentBranch {
            cwd: project_dir.to_path_buf(),
        })
        .ok()?;
    if !out.success() {
        return None;
    }
    let name = out.trimmed().trim().to_owned();
    if name.is_empty() { None } else { Some(name) }
}

/// Recognise the one `gh` failure that is a fact about the machine.
///
/// Same shape and same caution as [`classify_commit_failure`], including the
/// `None`-is-a-real-failure default. `gh` exits `1` for everything, so the
/// message is again the only signal — and `NO_COLOR` plus `LC_ALL=C` in the
/// constructed environment are what make it stable enough to match on.
///
/// The phrasings cover `gh`'s two shapes for "no credential": the interactive
/// one that tells you to log in, and the CI one that tells you to set a token.
/// Both are the same fact.
fn classify_gh_failure(out: &crate::exec::CommandOutput) -> Option<RemoteBlocked> {
    let haystack = format!("{} {}", out.stdout, out.stderr).to_ascii_lowercase();
    let unauthenticated = [
        "gh auth login",
        "not logged in",
        "no authentication token",
        "authentication token not found",
        "gh_token",
        "github_token",
    ];
    if unauthenticated.iter().any(|m| haystack.contains(m)) {
        return Some(RemoteBlocked::NotAuthenticated);
    }
    None
}

/// Recognise the two `git commit` failures that are facts about the machine.
///
/// Matching on `git`'s prose is not something to do lightly - it is
/// locale-dependent in general, which is why `LC_ALL=C` is set in the
/// constructed environment (see [`crate::exec`]). `git` offers no exit code that
/// distinguishes these, so the message is the only signal available, and
/// returning `None` for anything unrecognised keeps the default conservative:
/// an unknown failure stays a failure.
fn classify_commit_failure(out: &crate::exec::CommandOutput) -> Option<CommitBlocked> {
    let haystack = format!("{} {}", out.stdout, out.stderr).to_ascii_lowercase();
    if haystack.contains("nothing to commit") || haystack.contains("nothing added to commit") {
        return Some(CommitBlocked::NothingToCommit);
    }
    if haystack.contains("author identity unknown")
        || haystack.contains("please tell me who you are")
        || haystack.contains("empty ident name")
    {
        return Some(CommitBlocked::NoAuthorIdentity);
    }
    None
}

fn run(exec: &dyn ProcessRunner, op: &GitOp) -> Result<crate::exec::CommandOutput, CoreError> {
    let out = exec
        .run_git_op(op)
        .map_err(|e| CoreError::GitUnavailable { why: e.to_string() })?;
    if out.success() {
        return Ok(out);
    }
    Err(CoreError::ToolFailed {
        argv: out.argv.clone(),
        status: out.status,
        stderr: out.stderr.trim().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The break that lands beside `ALL`.**
    ///
    /// The `match` is exhaustive, so adding a variant stops this file
    /// compiling — and the fix is two lines above, in `ALL`. The length
    /// assertion is a literal for the same reason: it cannot be derived, so it
    /// is at least made to demand attention rather than to be forgotten
    /// quietly.
    ///
    /// This does **not** prove `ALL` is complete. Nothing available on stable
    /// does, short of a macro this project declined at two call sites
    /// (ADR-0001 §4). It puts the compiler's objection next to the list that
    /// needs editing, which is the strongest link there is here.
    #[test]
    fn all_lists_every_variant_and_every_variant_has_a_key() {
        for v in RemoteBlocked::ALL {
            match v {
                RemoteBlocked::GhMissing
                | RemoteBlocked::NotAuthenticated
                | RemoteBlocked::NothingToPush => {}
            }
        }
        assert_eq!(
            RemoteBlocked::ALL.len(),
            3,
            "a variant was added: extend ALL, then update this count"
        );

        // Keys are identifiers, not sentences, and they are distinct — a
        // duplicate would make two reasons indistinguishable to a frontend.
        let keys: Vec<&str> = RemoteBlocked::ALL.iter().map(|b| b.key()).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "duplicate key: {keys:?}");
        for k in keys {
            assert!(
                !k.contains(' '),
                "`{k}` reads like prose; keys are identifiers (ADR-0001 §4)"
            );
        }
    }

    #[test]
    fn a_branch_that_cannot_be_read_is_none_not_main() {
        // The substitution this project forbids, asserted at the one place it
        // would be tempting: an unknown branch printed as `main` would produce
        // a paste-ready command that is wrong.
        let dir = tempfile::tempdir().unwrap();
        let runner = crate::exec::NoRunner;
        assert_eq!(current_branch(dir.path(), &runner), None);
    }

    fn out(s: &str) -> crate::exec::CommandOutput {
        crate::exec::CommandOutput {
            argv: vec!["git".into(), "commit".into()],
            status: Some(1),
            stdout: String::new(),
            stderr: s.to_owned(),
        }
    }

    /// A runner that claims `gh` exists and **panics if anything runs it**.
    ///
    /// The pre-flight checks in [`create_remote`] are only worth anything if
    /// they run before `gh` does. Asserting on a returned reason would pass
    /// against an implementation that created the repository first and reported
    /// the reason afterwards, which is precisely the debris the checks exist to
    /// avoid. So the assertion is that the subprocess never happens.
    #[derive(Debug)]
    struct GhMustNotRun;

    impl ProcessRunner for GhMustNotRun {
        fn git_available(&self) -> bool {
            true
        }
        fn run_git(
            &self,
            _cwd: &Path,
            _args: &[&str],
        ) -> Result<crate::exec::CommandOutput, crate::detect::DetectError> {
            Err(crate::detect::DetectError::NotAttempted {
                why: "not under test".to_owned(),
            })
        }
        fn run_git_op(
            &self,
            _op: &GitOp,
        ) -> Result<crate::exec::CommandOutput, crate::detect::DetectError> {
            Err(crate::detect::DetectError::NotAttempted {
                why: "not under test".to_owned(),
            })
        }
        fn run_gh_op(
            &self,
            op: &GhOp,
        ) -> Result<crate::exec::CommandOutput, crate::detect::DetectError> {
            panic!("gh was run despite a pre-flight check that should have stopped it: {op:?}");
        }
        fn gh_available(&self) -> bool {
            true
        }
    }

    #[test]
    fn nothing_to_push_stops_before_gh_creates_an_empty_repository() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = create_remote(
            dir.path(),
            &GhMustNotRun,
            RepoVisibility::Private,
            false, // nothing was committed
            true,  // and gh is available, so only the check can stop this
        )
        .expect("a blocked remote is not an error");
        assert!(!outcome.created);
        assert_eq!(outcome.blocked, Some(RemoteBlocked::NothingToPush));
    }

    #[test]
    fn a_missing_gh_is_reported_and_never_attempted() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = create_remote(
            dir.path(),
            &crate::exec::NoRunner,
            RepoVisibility::Public,
            true,
            false,
        )
        .expect("a blocked remote is not an error");
        assert_eq!(outcome.blocked, Some(RemoteBlocked::GhMissing));
        assert!(outcome.url.is_none());
    }

    /// The half that keeps the two tests above honest: with both preconditions
    /// satisfied the op *is* run. Without this, an implementation that never
    /// called `gh` at all would satisfy every assertion here.
    #[test]
    fn with_both_preconditions_met_the_op_is_actually_run() {
        let dir = tempfile::tempdir().unwrap();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = create_remote(
                dir.path(),
                &GhMustNotRun,
                RepoVisibility::Private,
                true,
                true,
            );
        }))
        .expect_err("gh should have been invoked");
        let msg = err
            .downcast_ref::<String>()
            .map_or("", String::as_str)
            .to_owned();
        assert!(msg.contains("gh was run"), "unexpected panic: {msg}");
        assert!(msg.contains("RepoCreate"), "the op reaching gh was {msg}");
    }

    #[test]
    fn an_unauthenticated_gh_is_a_machine_fact_and_anything_else_is_a_failure() {
        for auth in [
            "To get started with GitHub CLI, please run: gh auth login.",
            "gh: To use GitHub CLI in a GitHub Actions workflow, set the GH_TOKEN environment variable",
            "You are not logged into any GitHub hosts",
        ] {
            assert_eq!(
                classify_gh_failure(&out(auth)),
                Some(RemoteBlocked::NotAuthenticated),
                "{auth}"
            );
        }

        // Paired, and this is the half that matters: an unrecognised failure
        // must stay a failure, so `gh`'s own explanation reaches the user
        // instead of being flattened into "no remote".
        for real in [
            "GraphQL: Name already exists on this account (createRepository)",
            "a git remote named 'origin' already exists",
            "failed to push: remote rejected",
            "error connecting to api.github.com",
        ] {
            assert_eq!(classify_gh_failure(&out(real)), None, "{real}");
        }
    }

    #[test]
    fn the_two_machine_facts_are_recognised_and_nothing_else_is() {
        assert_eq!(
            classify_commit_failure(&out("nothing to commit, working tree clean")),
            Some(CommitBlocked::NothingToCommit)
        );
        assert_eq!(
            classify_commit_failure(&out("Author identity unknown")),
            Some(CommitBlocked::NoAuthorIdentity)
        );
        assert_eq!(
            classify_commit_failure(&out("fatal: empty ident name not allowed")),
            Some(CommitBlocked::NoAuthorIdentity)
        );

        // Paired, and this half is the one that matters: an unrecognised
        // failure must stay a failure. A catch-all here would turn every commit
        // problem into a silent "not committed".
        for real in [
            "error: pathspec did not match any files",
            "fatal: not a git repository",
            "error: gpg failed to sign the data",
            "fatal: cannot lock ref HEAD",
        ] {
            assert_eq!(classify_commit_failure(&out(real)), None, "{real}");
        }
    }

    #[test]
    fn a_missing_author_identity_is_something_left_to_do_not_a_failure() {
        // A fresh machine with no `user.email` is common, and it is not ours to
        // fix: choosing a name and address on the user's behalf would stamp an
        // invented person into their history.
        let r = RepoReport {
            commit_blocked: Some(CommitBlocked::NoAuthorIdentity),
            committed: false,
            ..local_only()
        };
        assert!(r.needs_manual_finish(), "the user has to act, so say so");

        // Whereas nothing-to-commit is genuinely nothing to report.
        let quiet = RepoReport {
            commit_blocked: Some(CommitBlocked::NothingToCommit),
            ..r.clone()
        };
        assert!(!quiet.needs_manual_finish());
    }

    /// A local-only run: `--git` with no visibility flag, which is what the
    /// flag on its own means.
    fn local_only() -> RepoReport {
        RepoReport {
            initialised: true,
            already_a_repository: false,
            committed: true,
            commit_blocked: None,
            branch: Some("trunk".to_owned()),
            gh_available: false,
            remote_requested: None,
            remote_created: false,
            remote_blocked: None,
            remote_url: None,
        }
    }

    /// **The change of meaning worth a test of its own.** A missing `gh` used
    /// to make every `vibe new --git` report unfinished business. It now does
    /// so only when a remote was asked for: a limitation nobody reached is not
    /// something left to do, and reporting it anyway is the tool nagging about
    /// a problem the user does not have.
    #[test]
    fn a_missing_gh_is_only_unfinished_business_if_a_remote_was_asked_for() {
        let local = local_only();
        assert!(
            !local.needs_manual_finish(),
            "a local-only run reported unfinished business about a remote \
             nobody asked for"
        );

        // Paired: the same machine, the same missing `gh`, and a remote that
        // *was* requested.
        let asked = RepoReport {
            remote_requested: Some(RepoVisibility::Private),
            remote_blocked: Some(RemoteBlocked::GhMissing),
            ..local_only()
        };
        assert!(asked.needs_manual_finish());
    }

    #[test]
    fn a_created_remote_leaves_nothing_to_finish() {
        let done = RepoReport {
            gh_available: true,
            remote_requested: Some(RepoVisibility::Public),
            remote_created: true,
            remote_url: Some("https://github.com/you/demo.git".to_owned()),
            ..local_only()
        };
        assert!(!done.needs_manual_finish());

        // And the failure direction: asked for, `gh` present, not created.
        let failed = RepoReport {
            gh_available: true,
            remote_requested: Some(RepoVisibility::Public),
            remote_created: false,
            remote_blocked: Some(RemoteBlocked::NotAuthenticated),
            ..local_only()
        };
        assert!(failed.needs_manual_finish());
    }
}
