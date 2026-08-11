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
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CommitBlocked::NothingToCommit => "nothing to commit",
            CommitBlocked::NoAuthorIdentity => "git has no author identity configured",
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
}

impl RepoReport {
    /// Whether the user has anything left to do.
    #[must_use]
    pub fn needs_manual_finish(&self) -> bool {
        !self.gh_available || self.commit_blocked == Some(CommitBlocked::NoAuthorIdentity)
    }
}

/// Initialise a repository and commit the scaffold.
///
/// Idempotent in the direction that matters: an existing repository is left
/// alone rather than re-initialised, and a scaffold that is already committed
/// produces no second commit.
pub fn init(
    project_dir: &Path,
    exec: &dyn ProcessRunner,
    message: &str,
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

    Ok(RepoReport {
        initialised: !already,
        already_a_repository: already,
        committed,
        commit_blocked,
        // Read after the commit: before the first commit `HEAD` is unborn, and
        // `symbolic-ref` still resolves it, but reading it afterwards is the
        // state the user will actually be pushing.
        branch: current_branch(project_dir, exec),
        gh_available: gh_available(),
    })
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

/// Whether `gh` can be run. Probed the same way `git` is.
#[must_use]
pub fn gh_available() -> bool {
    std::process::Command::new("gh")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
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
            initialised: true,
            already_a_repository: false,
            committed: false,
            commit_blocked: Some(CommitBlocked::NoAuthorIdentity),
            branch: Some("trunk".to_owned()),
            gh_available: true,
        };
        assert!(r.needs_manual_finish(), "the user has to act, so say so");

        // Whereas nothing-to-commit is genuinely nothing to report.
        let quiet = RepoReport {
            commit_blocked: Some(CommitBlocked::NothingToCommit),
            ..r.clone()
        };
        assert!(!quiet.needs_manual_finish());
    }

    #[test]
    fn the_report_says_what_is_left_without_saying_it_about_the_project() {
        let r = RepoReport {
            initialised: true,
            already_a_repository: false,
            committed: true,
            commit_blocked: None,
            branch: Some("trunk".to_owned()),
            gh_available: false,
        };
        assert!(r.needs_manual_finish());

        let with_gh = RepoReport {
            gh_available: true,
            ..r.clone()
        };
        assert!(!with_gh.needs_manual_finish());
    }
}
