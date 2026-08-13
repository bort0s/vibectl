//! The ignore-state instrument, against the `git` that is actually installed.
//!
//! `src/ignore_state.rs`'s own tests cover the mapping over synthesised
//! outcomes, which is the half that can run anywhere. This file covers the half
//! that cannot be synthesised without begging the question: that a real `git`
//! produces the statuses and the stderr the mapping keys on, and that a real
//! *absence* of `git` produces the `Err` the mapping reads as `unknown`.
//!
//! # Every control here is paired, and the pairing is the whole point
//!
//! ADR-0010 §10 is explicit about it, because an implementation that returns
//! `unknown` unconditionally — a broken invocation, a swallowed error, a state
//! machine wired to one arm — satisfies every one-sided "this is unknown"
//! assertion perfectly, and the green would mean nothing. So each control has a
//! partner that must come out the *other* way through the identical path:
//!
//! | Control | Partner |
//! | --- | --- |
//! | ignore rule present → `ignored` | rule removed → `not ignored` |
//! | `PATH` without `git` → `unknown` | same input, `git` present → a real answer |
//! | exit `128` → `unknown` | exit `1` through the same runner → `not ignored` |
//! | `core.fsmonitor` not spawned by us | the same fixture spawns it without our environment |
//! | `core.excludesFile` honoured | the same file, empty `HOME`, is not ignored |
//!
//! # The subject is our code, not a reconstruction of it
//!
//! Two scenarios need a process environment this test cannot set in-process —
//! a `PATH` with no `git`, and a hostile `HOME`. `std::env::set_var` is
//! `unsafe` under edition 2024 and racy across parallel tests, so instead this
//! binary **re-executes itself**: the parent builds the environment and spawns
//! `current_exe()` filtered to [`child_probe_do_not_run_directly`], and the
//! child runs the real [`vibe_core::check_ignore`] through a real
//! [`SystemRunner`].
//!
//! That matters more than the convenience. A test that rebuilt the crate's
//! environment with its own `Command` would be asserting things about *its
//! copy* of the environment construction — the harness-versus-subject
//! disagreement (ADR-0002 §7) — and would keep passing on the day `child_env`
//! changed. Here the environment under test is the one `exec.rs` builds.

use std::path::{Path, PathBuf};
use std::process::Command;

use vibe_core::exec::{ProcessRunner, SystemRunner};
use vibe_core::git::GitOp;
use vibe_core::ignore_state::{IgnoreState, UnknownCause, check_ignore};

/// A program name that cannot exist on any machine, so `git`'s own error names
/// it and nothing has to run, exist, or win a race for the control to fire.
const FSMONITOR_PROBE: &str = "vibe-nonexistent-fsmonitor-probe";

/// Whether these controls can run here — and a hard failure where they must.
///
/// The `VIBE_REQUIRE_GH` shape (ADR-0002 §7), applied to `git`: a skipped test
/// and a passing test are the same green tick, so CI sets `VIBE_REQUIRE_GIT=1`
/// and a missing `git` becomes a failure rather than a shrug.
///
/// # What the green step proves, which is narrower than it reads
///
/// **It proves** this target exists, compiles and passes — `cargo test --test
/// ignore_state_git` exits `101` if it is deleted or renamed — and that every
/// call to this function which *executed* found `git`, because a missing one
/// panics.
///
/// **It does not prove that any assertion in this file ran.** Zero calls to
/// this function and one hundred are the same green. That is measured, not
/// cautious: ADR-0008 §9 sabotaged the two `gh` control files to return before
/// this same guard and the step stayed green on all three runners.
/// `VIBE_REQUIRE_GIT` closes an environment-shaped hole — a runner without
/// `git` silently voiding these controls — and not a code-shaped one, because
/// an exit status has nothing to observe about what a test asserted.
fn git_available() -> bool {
    let present = Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !present && std::env::var_os("VIBE_REQUIRE_GIT").is_some() {
        panic!(
            "VIBE_REQUIRE_GIT is set but `git` is not on PATH. These controls are \
             only meaningful where git exists, so refusing to run is reported as \
             a failure rather than a skip (ADR-0002 §7)."
        );
    }
    present
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(path, contents).expect("write");
}

/// A repository with one ignored file and one that is not.
///
/// `git init` is run through this crate's own [`GitOp`], not through a raw
/// `Command`: if the constructed environment could not create a repository,
/// every assertion below would be measuring a fixture our own code cannot
/// produce.
fn a_repository(root: &Path) {
    let exec = SystemRunner::default();
    exec.run_git_op(&GitOp::Init {
        cwd: root.to_path_buf(),
    })
    .expect("git init runs");
    write(&root.join(".gitignore"), "secret.md\n");
    write(&root.join("secret.md"), "private\n");
    write(&root.join("plain.md"), "public\n");
    write(&root.join(".claude/commands/daily.md"), "a prompt\n");
}

fn cause_key(state: &IgnoreState) -> String {
    state
        .cause()
        .map_or_else(|| "-".to_owned(), |c| c.key().to_owned())
}

// --- the mechanism control ------------------------------------------------

/// **Breaking the ignore rule must flip the state.**
///
/// ADR-0010 §10's first requirement: a test that reads `not ignored` from a
/// fixture where everything is ignored proves nothing about the mechanism. So
/// the same file is asked about twice, and the only thing that changes between
/// the two answers is the rule.
///
/// It also satisfies §10's second requirement — *the control must be reached* —
/// without a separate assertion. If `git` failed for its own reasons before the
/// state was computed, the first half would be `unknown` rather than `ignored`,
/// and the unreached guard would be a red test instead of a green one.
#[test]
fn breaking_the_ignore_rule_flips_the_state() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    a_repository(root);
    let exec = SystemRunner::default();

    let before = check_ignore(root, Path::new("secret.md"), &exec);
    assert_eq!(
        before,
        IgnoreState::Ignored,
        "the fixture's own ignore rule did not take effect, so nothing below \
         measures the mechanism"
    );

    // The only change.
    write(&root.join(".gitignore"), "# nothing is excluded any more\n");

    let after = check_ignore(root, Path::new("secret.md"), &exec);
    assert_eq!(after, IgnoreState::NotIgnored);
}

/// The state a `.claude/commands/` prompt has in a repository with no rule for
/// it — which under ADR-0010 §4's polarity B is the state that means *exposed*.
#[test]
fn a_prompt_with_no_rule_covering_it_is_not_ignored() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    a_repository(tmp.path());

    assert_eq!(
        check_ignore(
            tmp.path(),
            Path::new(".claude/commands/daily.md"),
            &SystemRunner::default(),
        ),
        IgnoreState::NotIgnored
    );
}

// --- the 128 pair ---------------------------------------------------------

/// **The second axis, which the `git`-absent pair never touches.**
///
/// Both halves of that pair stay inside the `Err`-versus-`Ok` split. The
/// collapse that matters is *inside* `Ok`: a `128` read as a `1`, an error
/// rendered as **not ignored**. So this is the same `git`, the same runner and
/// the same call, with one input that exits `128` and one that exits `1`
/// (ADR-0010 §10).
///
/// And the two `128` causes are asserted apart, not merely as `unknown`: git's
/// stderr distinguishes *not a repository* from *outside the repository*, the
/// remedies differ, and a test that only checked the state would pass against
/// an implementation that had merged them again.
#[test]
fn a_128_is_unknown_with_its_cause_while_a_1_through_the_same_call_is_not_ignored() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&repo).expect("mkdir");
    std::fs::create_dir_all(&outside).expect("mkdir");
    a_repository(&repo);
    write(&outside.join("elsewhere.md"), "not ours\n");
    let exec = SystemRunner::default();

    // 128, cause one: there is no repository here at all.
    let no_repo = check_ignore(&outside, Path::new("elsewhere.md"), &exec);
    assert_eq!(
        no_repo.cause(),
        Some(&UnknownCause::NotARepository),
        "got {no_repo:?}"
    );

    // 128, cause two: there is a repository, and the path is not in it.
    let elsewhere = check_ignore(&repo, &outside.join("elsewhere.md"), &exec);
    assert_eq!(
        elsewhere.cause(),
        Some(&UnknownCause::PathOutsideRepository),
        "got {elsewhere:?}"
    );

    // The partner. Same git, same runner, same function — and a real answer,
    // so the two assertions above are about `128` rather than about an
    // instrument that never says anything.
    assert_eq!(
        check_ignore(&repo, Path::new("plain.md"), &exec),
        IgnoreState::NotIgnored
    );
    assert_eq!(
        check_ignore(&repo, Path::new("secret.md"), &exec),
        IgnoreState::Ignored
    );
}

/// The trap in [`UnknownCause::PathOutsideRepository`], asserted so it is a
/// documented property rather than a surprise: git rejects a pathspec with a
/// leading `..` on its **text**, not on where it resolves.
#[test]
fn a_path_that_leaves_and_returns_is_still_reported_outside() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    a_repository(&repo);

    let state = check_ignore(
        &repo,
        Path::new("../repo/plain.md"),
        &SystemRunner::default(),
    );
    assert_eq!(
        state.cause(),
        Some(&UnknownCause::PathOutsideRepository),
        "got {state:?}"
    );
    // And the same file, named without the detour, answers normally — so this
    // is a fact about the pathspec and not about the file.
    assert_eq!(
        check_ignore(&repo, Path::new("plain.md"), &SystemRunner::default()),
        IgnoreState::NotIgnored
    );
}

/// The argv [`GitOp::CheckIgnore`] constructs is argv this `git` still accepts.
///
/// Taken from the enum rather than retyped: a test that spells the arguments
/// out again is testing its own copy. `129` — the usage error measured from an
/// unknown flag — is the failure this would show up as, and it would arrive as
/// `Unrecognised` rather than as anything alarming, which is exactly why it
/// needs asserting separately from the state.
#[test]
fn this_git_still_accepts_the_argv_the_enum_constructs() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    a_repository(tmp.path());

    let op = GitOp::CheckIgnore {
        cwd: tmp.path().to_path_buf(),
        path: PathBuf::from("secret.md"),
    };
    let out = SystemRunner::default().run_git_op(&op).expect("git runs");

    assert_eq!(
        out.status,
        Some(0),
        "argv {:?} was not accepted: {}",
        op.argv(),
        out.stderr
    );
    assert!(
        out.stderr.is_empty(),
        "a clean invocation wrote to stderr: {}",
        out.stderr
    );
}

// --- the 4a controls, through the environment this crate constructs -------

/// **`core.fsmonitor` is not spawned, and `core.excludesFile` still is
/// honoured.**
///
/// The two questions ADR-0005 §4a was amended to ask, measured together because
/// separating them is how a correct application of the rule destroys the
/// answer. A neutralisation at the granularity of *the whole config* would pass
/// the first assertion and fail the second, while looking like more security.
///
/// The hazard half is demonstrated rather than assumed: the same planted config
/// really does spawn the probe when `git` runs without our environment. Without
/// that, "the probe did not appear" is indistinguishable from a fixture that
/// never armed.
#[test]
fn the_planted_config_cannot_execute_but_can_still_answer() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    a_repository(&repo);

    // `plain.md` is the control file: no rule in the repository mentions it, so
    // the only thing that can make it ignored is the redirect.
    let hostile = tmp.path().join("home-hostile");
    write(&hostile.join("excludes.txt"), "plain.md\n");
    write(
        &hostile.join(".gitconfig"),
        &format!(
            "[core]\n\tfsmonitor = {FSMONITOR_PROBE}\n\texcludesFile = {}\n\tpager = vibe-nonexistent-pager-probe\n\
             [alias]\n\tcheck-ignore = !vibe-nonexistent-alias-probe\n",
            hostile
                .join("excludes.txt")
                .display()
                .to_string()
                .replace('\\', "/")
        ),
    );
    // The control: byte-identical invocation, no such config.
    let plain_home = tmp.path().join("home-plain");
    std::fs::create_dir_all(&plain_home).expect("mkdir");

    // First: is the hazard actually present in this fixture? Run git the way
    // *nothing in this crate* runs it — no neutralisation — and require the
    // spawn error. This is the harness demonstrating the fixture is armed; the
    // assertions that follow are the subject showing it does not fire.
    let armed = Command::new("git")
        .args(["--no-pager", "check-ignore", "--", "plain.md"])
        .current_dir(&repo)
        .env("HOME", &hostile)
        .env("USERPROFILE", &hostile)
        .env("LC_ALL", "C")
        .output()
        .expect("git runs");
    let armed_stderr = String::from_utf8_lossy(&armed.stderr).into_owned();
    assert!(
        armed_stderr.contains(FSMONITOR_PROBE),
        "the planted config did not spawn anything on this git, so the \
         neutralisation below is being verified against a hazard that is not \
         present. That is not a reason to drop it — it is a reason to find out \
         why (git version, or `core.fsmonitor` handling, has moved). stderr: \
         {armed_stderr}"
    );

    // Now the subject: the same repository, the same planted `HOME`, reached
    // through this crate's own op and environment.
    let hostile_run = probe_in_child(&repo, Path::new("plain.md"), |cmd| {
        cmd.env("HOME", &hostile).env("USERPROFILE", &hostile);
    });

    assert!(
        !hostile_run.stderr.contains(FSMONITOR_PROBE),
        "core.fsmonitor was spawned through the environment this crate builds: {}",
        hostile_run.stderr
    );
    assert_eq!(
        hostile_run.state, "ignored",
        "core.excludesFile stopped being honoured, which is the redirect this \
         op exists to ask git about (cause: {})",
        hostile_run.cause
    );

    // The partner: the same file, the same code, no redirect. Without this the
    // assertion above would also pass against an implementation that reported
    // everything as ignored.
    let plain_run = probe_in_child(&repo, Path::new("plain.md"), |cmd| {
        cmd.env("HOME", &plain_home).env("USERPROFILE", &plain_home);
    });
    assert_eq!(plain_run.state, "not_ignored", "cause: {}", plain_run.cause);
}

// --- the git-absent pair --------------------------------------------------

/// **`PATH` without `git` yields `unknown`, and the same input with `git`
/// present yields a real answer.**
///
/// ADR-0010 §8 says the Rust-side observable for absence is a spawn error
/// rather than a status — an `Err` raised before any exit code exists — and
/// records that observable as *owed by this diff rather than asserted*. This is
/// the payment.
///
/// The fixture is constructed rather than hoped for (ADR-0002 §7: a
/// precondition you did not construct is not a precondition), and the assertion
/// is that the state is **`unknown`**, not merely that it is not `ignored`.
/// `unknown` and `not ignored` are one boolean apart and only one of them is a
/// lie.
#[test]
fn git_absent_is_unknown_and_git_present_answers_through_the_identical_path() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    a_repository(&repo);

    // A directory with nothing in it, as the entire PATH.
    let empty = tmp.path().join("no-git-here");
    std::fs::create_dir_all(&empty).expect("mkdir");

    let absent = probe_in_child(&repo, Path::new("secret.md"), |cmd| {
        cmd.env("PATH", &empty);
    });
    assert_eq!(
        absent.state, "unknown",
        "a missing git was reported as a fact about the file (cause: {}, stderr: {})",
        absent.cause, absent.stderr
    );
    assert_eq!(absent.cause, "git_not_run");

    // The partner, through the identical channel: same child, same fixture,
    // same file — with `PATH` left alone.
    let present = probe_in_child(&repo, Path::new("secret.md"), |_| {});
    assert_eq!(
        present.state, "ignored",
        "cause: {}, stderr: {}",
        present.cause, present.stderr
    );
}

// --- the no-exit-code arm, where it is genuinely reachable ----------------

/// **A real `SIGKILL`, gated to Unix.**
///
/// The mapping is covered on all three platforms by
/// `ignore_state::tests::no_exit_code_is_unknown_and_emphatically_not_not_ignored`,
/// which synthesises the value. That is the best available on Windows, where
/// there are no signals and a code is returned for essentially everything — but
/// filing a synthesised value as the best available on *all three* would be
/// calling something unreachable while it is in reach on two runners out of
/// three (ADR-0010 §10).
///
/// So this is the reachable half: a child really is killed by a signal, and the
/// `Option<i32>` handed to the classifier is the one that process produced.
/// Windows keeps mapping-only coverage, recorded as a declared platform limit
/// rather than as a choice.
#[cfg(unix)]
#[test]
fn a_signal_killed_child_produces_no_exit_code_and_maps_to_unknown() {
    use vibe_core::exec::CommandOutput;
    use vibe_core::ignore_state::classify;

    let status = Command::new("sh")
        .arg("-c")
        .arg("kill -9 $$")
        .status()
        .expect("sh runs");

    // The premise. If this ever became `Some(_)`, the arm below would be an
    // unreachable branch on this platform too, and the test would be asserting
    // a mapping over a value nothing produces.
    assert!(
        status.code().is_none(),
        "a SIGKILLed child reported an exit code ({status:?}), so this platform \
         no longer reaches the no-exit-code outcome"
    );

    let out = CommandOutput {
        argv: vec!["git".to_owned(), "check-ignore".to_owned()],
        status: status.code(),
        stdout: String::new(),
        // Empty, deliberately: with no exit code there is nothing for the
        // stderr classifier to fall back on, which is the worst case.
        stderr: String::new(),
    };

    let state = classify(Ok(out));
    assert_eq!(state.cause(), Some(&UnknownCause::NoExitCode));
    // `success()` is false here and false is also what exit 1 produces. This is
    // the assertion that separates them.
    assert_ne!(state, IgnoreState::NotIgnored);
}

// --- the re-execution machinery -------------------------------------------

/// What the child measured.
#[derive(Debug)]
struct Probe {
    state: String,
    cause: String,
    stderr: String,
}

/// Environment variables the child reads. Absent in an ordinary run, which is
/// how [`child_probe_do_not_run_directly`] knows it is not the child.
const CHILD_DIR: &str = "VIBE_IGNORE_PROBE_DIR";
const CHILD_PATH: &str = "VIBE_IGNORE_PROBE_PATH";
const CHILD_OUT: &str = "VIBE_IGNORE_PROBE_OUT";

/// Run [`check_ignore`] in a fresh process whose environment `configure` shapes.
///
/// The parent controls only the variables it means to control; everything else
/// is inherited, so the difference between two calls is the thing being tested
/// and not the rest of the environment drifting.
fn probe_in_child(dir: &Path, path: &Path, configure: impl FnOnce(&mut Command)) -> Probe {
    let out_dir = tempfile::tempdir().expect("tempdir");
    let out_file = out_dir.path().join("probe.txt");

    let mut cmd = Command::new(std::env::current_exe().expect("current exe"));
    cmd.args([
        "--exact",
        "child_probe_do_not_run_directly",
        "--test-threads=1",
        "--quiet",
    ])
    .env(CHILD_DIR, dir)
    .env(CHILD_PATH, path)
    .env(CHILD_OUT, &out_file);
    configure(&mut cmd);

    let output = cmd.output().expect("the test binary re-runs");
    let raw = std::fs::read_to_string(&out_file).unwrap_or_else(|e| {
        panic!(
            "the child wrote no result ({e}).\nstatus: {:?}\nstdout: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
    });

    // `state\ncause\nstderr…` — the stderr is last because it is the only
    // field that can contain newlines.
    let mut lines = raw.splitn(3, '\n');
    Probe {
        state: lines.next().unwrap_or_default().to_owned(),
        cause: lines.next().unwrap_or_default().to_owned(),
        stderr: lines.next().unwrap_or_default().to_owned(),
    }
}

/// The child half of [`probe_in_child`]. **Not a test**, despite the attribute:
/// it is how a `#[test]` binary is asked to run one function.
///
/// In an ordinary `cargo test` run the variables are absent and this returns
/// immediately — a green no-op. That is safe here in a way a skipped control
/// normally is not, because nothing asserts anything in this function: the
/// assertions live in the parents, and a parent whose child failed to write a
/// result fails with the child's output attached.
#[test]
fn child_probe_do_not_run_directly() {
    let (Some(dir), Some(path), Some(out)) = (
        std::env::var_os(CHILD_DIR),
        std::env::var_os(CHILD_PATH),
        std::env::var_os(CHILD_OUT),
    ) else {
        return;
    };
    let dir = PathBuf::from(dir);
    let path = PathBuf::from(path);
    let exec = SystemRunner::default();

    // The state, through the crate's public entry point.
    let state = check_ignore(&dir, &path, &exec);

    // And the raw stderr of the same invocation, which `check_ignore` discards
    // because it is not part of the answer — but which is where a spawned
    // `core.fsmonitor` would announce itself.
    let stderr = exec
        .run_git_op(&GitOp::CheckIgnore { cwd: dir, path })
        .map_or_else(|e| e.to_string(), |o| o.stderr);

    std::fs::write(
        PathBuf::from(out),
        format!("{}\n{}\n{stderr}", state.key(), cause_key(&state)),
    )
    .expect("the child writes its result");
}
