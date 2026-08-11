//! A guard on the scan budget that a CI runner can actually keep.
//!
//! Wall-clock assertions do not belong in CI: a shared runner's timings drift
//! by more than the thing being measured. Measured on one machine in one
//! session, the same binary and corpus produced a median of 583ms and then a
//! median of 684ms depending only on what else was running — a 17% shift, with
//! the spread widening 3.6x. A 2000ms assertion would pass while a 3x
//! regression hid inside it, and would fail spuriously on a loaded runner.
//!
//! So this asserts the *cause* instead. The scan is ~98% subprocess spawn:
//! walking and detecting 50 repositories costs ~93ms, while the `git` calls for
//! the same 50 cost ~570ms at a measured ceiling of ~175 spawns/second (the
//! knee is at core count — Windows process creation is kernel-CPU work, not
//! I/O wait). Subprocess count per project is therefore the budget, and it is
//! deterministic.
//!
//! If someone adds a third git call per repository, that is a ~50% regression
//! and this test says so on every platform, in every session.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use vibe_core::exec::{CommandOutput, ProcessRunner, SystemRunner};
use vibe_core::{Config, NullReporter, Registry, ScanRequest};

/// Wraps the real runner and counts what goes through it.
#[derive(Debug)]
struct CountingRunner {
    inner: SystemRunner,
    calls: AtomicUsize,
    argv: Mutex<Vec<Vec<String>>>,
}

impl CountingRunner {
    fn new() -> Self {
        Self {
            inner: SystemRunner::default(),
            calls: AtomicUsize::new(0),
            argv: Mutex::new(Vec::new()),
        }
    }
}

impl ProcessRunner for CountingRunner {
    fn git_available(&self) -> bool {
        self.inner.git_available()
    }

    fn run_git(
        &self,
        cwd: &Path,
        args: &[&str],
    ) -> Result<CommandOutput, vibe_core::detect::DetectError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.argv
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(args.iter().map(|s| (*s).to_owned()).collect());
        self.inner.run_git(cwd, args)
    }

    /// Counted like any other invocation, so a detector that starts reaching
    /// for a store op cannot slip past the scan budget by using the other
    /// entry point.
    fn run_git_op(
        &self,
        op: &vibe_core::agents::GitOp,
    ) -> Result<CommandOutput, vibe_core::detect::DetectError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.argv
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(op.argv());
        self.inner.run_git_op(op)
    }

    /// Counted for the same reason `run_git_op` is, and with more force: a scan
    /// has no business creating a repository on github.com, so an invocation
    /// reaching here at all would show up as a budget overrun rather than as
    /// nothing.
    fn run_gh_op(
        &self,
        op: &vibe_core::GhOp,
    ) -> Result<CommandOutput, vibe_core::detect::DetectError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.argv
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(op.argv());
        self.inner.run_gh_op(op)
    }

    fn gh_available(&self) -> bool {
        self.inner.gh_available()
    }
}

fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn make_repo(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("package.json"), r#"{"name":"x"}"#).expect("write");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@example.invalid"],
        vec!["config", "user.name", "t"],
        // No detached `git maintenance run --auto` per commit. This fixture
        // builds 50 repositories, so that is 50 background processes competing
        // with the very subprocess budget being measured. See the longer note
        // in `scan_never_writes.rs`, where the same behaviour was an
        // intermittent red.
        vec!["config", "gc.auto", "0"],
        vec!["config", "maintenance.auto", "false"],
        vec!["add", "package.json"],
        vec!["commit", "-q", "-m", "initial"],
    ] {
        // Asserted, not discarded: a fixture whose `git init` failed produces a
        // repository with no commit, and the failure then surfaces as "every
        // repo has a commit, so every one should report it" — which accuses the
        // scanner of a bug the fixture caused.
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(&dir)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "fixture step `git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// The budget, expressed as the quantity that determines it.
const MAX_GIT_CALLS_PER_PROJECT: usize = 2;

#[test]
fn a_scan_spends_at_most_two_git_invocations_per_repository() {
    if !git_available() {
        eprintln!("skipping: git is not on PATH");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..5 {
        make_repo(dir.path(), &format!("repo{i}"));
    }

    let runner = std::sync::Arc::new(CountingRunner::new());
    let registry = Registry::open(Config::default()).with_runner(runner.clone());
    let report = registry.scan(&ScanRequest::new(dir.path()), &NullReporter);

    assert_eq!(report.projects.len(), 5);
    let calls = runner.calls.load(Ordering::Relaxed);
    assert!(
        calls <= 5 * MAX_GIT_CALLS_PER_PROJECT,
        "{calls} git calls for 5 repositories exceeds the budget of \
         {MAX_GIT_CALLS_PER_PROJECT} per project. Subprocess spawn is ~98% of \
         scan time; every extra call per repo is a ~50% regression. Calls made: {:?}",
        runner
            .argv
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    );

    // The scan actually used git, or this proves nothing.
    assert!(calls > 0, "no git calls were made at all");
    assert!(
        report
            .projects
            .iter()
            .all(|p| p.detection.last_commit.is_known()),
        "every repo has a commit, so every one should report it"
    );
}

#[test]
fn every_git_invocation_declines_to_take_a_lock() {
    if !git_available() {
        eprintln!("skipping: git is not on PATH");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    make_repo(dir.path(), "repo");

    let runner = std::sync::Arc::new(CountingRunner::new());
    let registry = Registry::open(Config::default()).with_runner(runner.clone());
    let _ = registry.scan(&ScanRequest::new(dir.path()), &NullReporter);

    let argv = runner
        .argv
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(!argv.is_empty());
    for call in &argv {
        assert!(
            call.contains(&"--no-optional-locks".to_owned()),
            "a read must not be able to take the index lock: {call:?}"
        );
    }
}

/// A git worktree or submodule has `.git` as a *file*, not a directory. Before
/// this was handled, every such project reported no remote and no commit as
/// `no_evidence` — the tool claiming the repository had nothing to say when it
/// had simply never asked.
#[test]
fn a_git_worktree_is_recognised_as_a_repository() {
    if !git_available() {
        eprintln!("skipping: git is not on PATH");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    make_repo(dir.path(), "main");
    let wt = dir.path().join("wt");
    let ok = std::process::Command::new("git")
        .args(["worktree", "add", "-q"])
        .arg(&wt)
        .args(["-b", "feature"])
        .current_dir(dir.path().join("main"))
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("skipping: could not create a worktree");
        return;
    }

    assert!(
        wt.join(".git").is_file(),
        "the fixture must have a .git FILE"
    );

    let registry = Registry::open(Config::default());
    let report = registry.scan(&ScanRequest::new(&wt), &NullReporter);
    let p = report.projects.first().expect("the worktree is a project");

    assert!(
        p.detection.last_commit.is_known(),
        "a worktree shares the repository's history: {:?}",
        p.detection.last_commit.reason()
    );
}

/// The `%D` regression, guarded structurally.
///
/// `git log -1 --format=%cI%n%D` looks like a free way to get the branch along
/// with the commit date. It is not: `%D` is the ref decoration, so git loads
/// and matches every ref in the repository. Measured on this machine, that took
/// a flat 44ms call to 439ms at 2,000 refs and 4,567ms at 50,000 — one project
/// costing twice the budget for fifty. A repository reaches thousands of refs
/// simply by having a few hundred remote branches.
///
/// Wall-clock cannot catch this in CI, and a corpus of fresh repositories has
/// no refs to expose it. So the format string itself is the assertion.
#[test]
fn no_git_call_uses_a_ref_count_dependent_format() {
    if !git_available() {
        eprintln!("skipping: git is not on PATH");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    make_repo(dir.path(), "repo");

    let runner = std::sync::Arc::new(CountingRunner::new());
    let registry = Registry::open(Config::default()).with_runner(runner.clone());
    let _ = registry.scan(&ScanRequest::new(dir.path()), &NullReporter);

    let argv = runner
        .argv
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(!argv.is_empty(), "the scan should have used git");

    for call in &argv {
        for arg in call {
            assert!(
                !arg.contains("%D") && !arg.contains("%d"),
                "ref-decoration formats grow with ref count and must not be used: {call:?}"
            );
        }
    }
}
