//! `vibe new --git` through the binary.
//!
//! # Why these plant a `HOME` instead of asking the machine
//!
//! The first version of this file branched on whether the machine had a git
//! identity, probing with `git config user.email` in the test's own
//! environment. It passed on Linux and Windows and **failed on macOS**, and the
//! reason is the whole lesson: `vibe` runs `git` under `env_clear()` plus
//! `GIT_CONFIG_NOSYSTEM=1`, so the test and the code under test were measuring
//! two different things. A system-level `gitconfig` is visible to one and
//! invisible to the other.
//!
//! That is ADR-0002 §7's rule that **the harness and the subject must agree
//! about what environment is under test**: the failure named the subject and was
//! caused by the harness measuring a different world. The answer is not a better
//! probe — it is to stop reading ambient state at all. Each test plants a `HOME`
//! containing exactly the configuration it wants, so both halves of every pair
//! run everywhere rather than skipping on whichever machine happens to be
//! configured the other way.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A home directory containing precisely the git configuration named.
fn planted_home(root: &Path, gitconfig: Option<&str>) -> PathBuf {
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("mkdir home");
    // QUIET_MAINTENANCE unconditionally, including for the no-identity home:
    // the commit is refused there, but `git init` and `git add` still run, and
    // the rule is about the fixture rather than about whether this particular
    // run reaches the commit.
    //
    // With no identity requested, REFUSE_AUTODETECT is added too - see its
    // docs for why an absent identity is not by itself a deterministic state.
    let text = match gitconfig {
        Some(extra) => format!("{QUIET_MAINTENANCE}{extra}"),
        None => format!("{QUIET_MAINTENANCE}{REFUSE_AUTODETECT}"),
    };
    std::fs::write(home.join(".gitconfig"), text).expect("write gitconfig");
    home
}

/// Settings every planted home carries, whatever else it configures.
///
/// **ADR-0002 §7: a fixture must not leave anything running.** `git commit`
/// spawns a *detached* `git maintenance run --auto`, which outlives the command
/// that started it, takes a lock under `.git/`, and releases it whenever it
/// finishes. That is precisely the race `c8845a0` root-caused after it produced
/// an intermittent macOS red that named `vibe scan` and was nothing to do with
/// it.
///
/// The existing scan fixtures set these for that reason. This one commits too,
/// so it needs them too — and it is planted in the home `vibe`'s own `git` reads,
/// because `env_clear()` means a repo-local `git config` set by the test would
/// not be what the subprocess sees.
const QUIET_MAINTENANCE: &str = "[gc]\n\tauto = 0\n[maintenance]\n\tauto = false\n";

/// Force `git` to refuse rather than invent an identity.
///
/// **This is the platform-dependent variable the fixture has to remove.** With
/// no identity configured, `git` may *auto-detect* one from the login name and
/// the hostname and commit with a warning — and whether that succeeds depends
/// on whether the machine has a resolvable hostname. A CI container often does
/// not (`runner@fv-az123.(none)`) and the commit fails; a developer machine
/// usually does and it succeeds. Two platforms, two behaviours, one assertion.
///
/// `user.useConfigOnly` makes the refusal unconditional, and `git` emits the
/// same "Author identity unknown" it emits when auto-detection fails — so the
/// condition under test is the real one, produced deterministically rather than
/// hoped for.
const REFUSE_AUTODETECT: &str = "[user]\n\tuseConfigOnly = true\n";

const IDENTITY: &str = "[user]\n\tname = Test\n\temail = test@example.invalid\n";

/// Run `vibe new --git` with a controlled `HOME`.
fn new_git(root: &Path, name: &str, home: &Path) -> std::process::Output {
    new_git_with(root, name, home, &[])
}

/// The same, with extra flags.
///
/// # What is deliberately never tested here
///
/// No test in this file runs the path where `gh` **succeeds**. That path
/// creates a repository on github.com under whoever is logged in, and a test
/// suite that can do that on a developer's machine or a CI runner is a test
/// suite that will eventually do it by accident. The `gh` side is covered
/// where it can be covered without side effects: `gh_argv.rs` proves this
/// crate's argv is what `gh` accepts, `gh_containment.rs` proves a per-user
/// config cannot redirect it, and `repo.rs`'s unit tests prove the pre-flight
/// checks and the failure classification.
///
/// The `--private` test below is safe for two independent reasons, not one:
/// the planted `HOME` contains no `gh` credential, and `vibe` never forwards
/// `GH_TOKEN`, so `gh` cannot authenticate — and before either matters, the
/// commit is refused, so the remote step stops before `gh` is invoked at all.
fn new_git_with(root: &Path, name: &str, home: &Path, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vibe"))
        .args(["new", name, "--git"])
        .args(extra)
        .arg("--path")
        .arg(root)
        // Both, because git resolves the per-user config through `HOME` on unix
        // and `USERPROFILE` on Windows, and `vibe` forwards each.
        .env("HOME", home)
        .env("USERPROFILE", home)
        // `gh` looks here before `HOME` on unix. Planted for the same reason:
        // the test states the environment rather than inheriting one.
        .env("XDG_CONFIG_HOME", home.join("xdg"))
        .output()
        .expect("run vibe")
}

fn stdout(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn without_the_flag_no_repository_is_created() {
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_vibe"))
        .args(["new", "plain", "--path"])
        .arg(tmp.path())
        .output()
        .expect("run vibe");
    assert!(out.status.success());
    // Opt-in, not default: creating a repository unasked is the tool doing
    // something that was not requested (ADR-0008 §8).
    assert!(
        !tmp.path().join("plain/.git").exists(),
        "a repository was created without --git"
    );
}

/// **The remote is a second opt-in, and the CLI refuses to guess.**
///
/// `--private`/`--public` without `--git` is refused by argument parsing, so
/// there is no reading of "create the remote" that does not also say "make a
/// repository". And the two visibilities conflict, so `vibe` is never asked to
/// pick.
#[test]
fn asking_for_a_remote_without_a_repository_is_refused_before_anything_runs() {
    let tmp = tempfile::tempdir().unwrap();
    for flags in [
        vec!["new", "demo", "--private", "--path"],
        vec!["new", "demo", "--public", "--path"],
        vec!["new", "demo", "--git", "--private", "--public", "--path"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_vibe"))
            .args(&flags)
            .arg(tmp.path())
            .output()
            .expect("run vibe");
        assert!(!out.status.success(), "{flags:?} was accepted");
        assert!(
            !tmp.path().join("demo").exists(),
            "{flags:?} scaffolded before failing"
        );
    }
}

/// **The behaviour ADR-0008 §3 was amended for.** `--git` on its own is a
/// request for a local repository. On a machine where `gh` is missing it used
/// to print the finish-the-job advice regardless, which is a limitation nobody
/// reached; on a machine where `gh` is present, the alternative reading — create
/// the remote because we can — would publish a repository from a flag whose
/// help text says "initialise a git repository".
///
/// Deterministic on every machine precisely because the answer no longer
/// depends on whether `gh` is installed.
#[test]
fn the_flag_alone_says_nothing_about_remotes_whether_or_not_gh_exists() {
    if !git_available() {
        eprintln!("skipping: git is not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let home = planted_home(tmp.path(), Some(IDENTITY));
    let out = new_git(tmp.path(), "demo", &home);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let text = stdout(&out);
    assert!(text.contains("Committed the scaffold"), "{text}");
    let lower = text.to_lowercase();
    for volunteered in ["gh ", "remote", "push"] {
        assert!(
            !lower.contains(volunteered),
            "`--git` volunteered `{volunteered}` for a remote nobody asked \
             for:\n{text}"
        );
    }
}

/// With nothing committed there is nothing to push, and `gh` is not run at all.
///
/// The message is the evidence: if `gh` had been invoked, the report would name
/// what `gh` said — that it is missing, or that it cannot authenticate — and
/// those are different sentences from this one.
#[test]
fn a_requested_remote_stops_before_gh_when_there_is_no_commit() {
    if !git_available() {
        eprintln!("skipping: git is not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // No identity, so the commit is refused deterministically (see
    // REFUSE_AUTODETECT) and the remote step has nothing to push.
    let home = planted_home(tmp.path(), None);
    let out = new_git_with(tmp.path(), "demo", &home, &["--private"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let text = stdout(&out);
    assert!(text.contains("no commit to push"), "{text}");
    assert!(text.contains("will not invent one"), "{text}");
    // The two things it would have said had it run `gh`.
    assert!(!text.contains("gh was not found"), "{text}");
    assert!(!text.contains("not authenticated"), "{text}");
}

#[test]
fn the_flag_initialises_a_repository_and_reports_the_real_branch() {
    if !git_available() {
        eprintln!("skipping: git is not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let home = planted_home(tmp.path(), Some(IDENTITY));
    let out = new_git(tmp.path(), "demo", &home);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let proj = tmp.path().join("demo");
    assert!(proj.join(".git").exists(), "no repository");

    // The branch named in the output must be the branch `git` actually has.
    // Reading it back rather than printing `main` is the point: the message
    // exists to be pasted, so a wrong branch in it is worse than none.
    let actual = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(&proj)
        .output()
        .expect("git runs");
    let branch = String::from_utf8_lossy(&actual.stdout).trim().to_owned();
    assert!(!branch.is_empty(), "fixture: no branch to compare against");

    let text = stdout(&out);
    assert!(
        text.contains(&branch),
        "the report must name the real branch `{branch}`:\n{text}"
    );
}

/// A branch name the tool could not have guessed.
///
/// `init.defaultBranch` is user configuration. If the readback were replaced by
/// a hard-coded `main`, this is the test that catches it — which a project
/// initialised on the default never could.
#[test]
fn a_non_default_branch_name_is_reported_not_assumed() {
    if !git_available() {
        eprintln!("skipping: git is not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let home = planted_home(
        tmp.path(),
        Some(&format!("{IDENTITY}[init]\n\tdefaultBranch = quincunx\n")),
    );
    let out = new_git(tmp.path(), "demo", &home);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let text = stdout(&out);
    assert!(
        text.contains("quincunx"),
        "the branch was assumed rather than read:\n{text}"
    );
    assert!(
        !text.contains("origin main"),
        "a hard-coded `main` leaked into the paste-ready command:\n{text}"
    );
}

/// A machine with no author identity is common, and `vibe` must neither invent
/// one nor fail.
#[test]
fn a_missing_author_identity_is_reported_and_never_invented() {
    if !git_available() {
        eprintln!("skipping: git is not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // A home with no identity, and with auto-detection refused so that
    // "no identity" means the same thing on every platform. `vibe` also sets
    // GIT_CONFIG_NOSYSTEM=1, so no system-level identity can leak in either.
    let home = planted_home(tmp.path(), None);
    let out = new_git(tmp.path(), "demo", &home);

    // Not a failure. A missing identity is a fact about the machine.
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = stdout(&out);
    assert!(text.contains("will not invent one"), "{text}");
    assert!(text.contains("git config --global user.email"), "{text}");

    // And nothing was committed under a fabricated author.
    let log = Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(tmp.path().join("demo"))
        .output()
        .expect("git runs");
    assert!(
        !log.status.success() || log.stdout.is_empty(),
        "a commit was made despite no author identity: {}",
        String::from_utf8_lossy(&log.stdout)
    );
}

/// The paired half, and the one that makes the test above non-vacuous: with an
/// identity present the scaffold really is committed, and the advice is absent.
/// Asserting only the missing case would pass against a build that printed the
/// advice unconditionally and never committed anything.
#[test]
fn with_an_identity_the_scaffold_is_committed_and_no_advice_is_printed() {
    if !git_available() {
        eprintln!("skipping: git is not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let home = planted_home(tmp.path(), Some(IDENTITY));
    let out = new_git(tmp.path(), "demo", &home);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let text = stdout(&out);
    assert!(text.contains("Committed the scaffold"), "{text}");
    assert!(
        !text.contains("will not invent one"),
        "identity advice printed when an identity exists:\n{text}"
    );

    let log = Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(tmp.path().join("demo"))
        .output()
        .expect("git runs");
    assert!(log.status.success() && !log.stdout.is_empty(), "no commit");
}
