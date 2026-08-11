//! **ADR-0008 §2.** Does the argv [`GhOp`] constructs mean, to a real `gh`,
//! what this crate believes it means?
//!
//! The closed enum and the argument allowlist are asserted against **strings**
//! by unit tests: they prove that no variant can express `alias set`, and that
//! an unlisted flag is refused. Neither can tell whether `--source=.` is still
//! a flag `gh` has. That question has one honest answer — ask `gh` — and it is
//! the same reason ADR-0005 §10 rule 4's local-path form exists rather than a
//! mocked command runner: a control that never reaches the thing it controls
//! for is not a control.
//!
//! # Why this can run at all, given it must create nothing
//!
//! `--help` is inserted **after the subcommand and before the flags**, so
//! `cobra` parses every flag this crate emits and then prints help instead of
//! running anything. Flag parsing happens first, so an unknown flag is still an
//! error — which is exactly the failure this is here to catch.
//!
//! Three independent reasons nothing can be created, because one would be a
//! claim and three are a design:
//!
//! 1. `--help` short-circuits before `gh` does any work.
//! 2. `GH_CONFIG_DIR` points at an empty directory this test made, and no token
//!    is forwarded, so `gh` has no credential to create anything with.
//! 3. The working directory is not a git repository, so `--source=.` has
//!    nothing to push.
//!
//! # Paired, per ADR-0002 §7
//!
//! Asserting only that the real argv parses is one-sided: it would keep passing
//! against a `gh` that stopped rejecting unknown flags, and against a test that
//! was silently looking at the wrong output. So the same invocation runs twice,
//! differing in one element — a flag `gh` does not have — and the rejection must
//! appear in one and be absent from the other.
//!
//! # What this control does *not* claim
//!
//! It exercises argv, not the environment. The constructed environment is
//! asserted in `exec.rs`'s unit tests, which can read the map directly rather
//! than infer it from a subprocess's behaviour. Splitting them keeps each
//! assertion about a thing it can actually see.

use std::path::Path;
use std::process::Command;

use vibe_core::gh::{GhOp, RepoVisibility};

/// Whether this control can run here — and a hard failure where it must.
///
/// The `VIBE_REQUIRE_GH` shape, identical to `gh_containment.rs` and for the
/// identical reason: a skipped test and a passing test are the same green tick,
/// so CI sets the variable and a missing `gh` becomes a failure rather than a
/// shrug. **The step passing is itself the proof the control ran** — no log
/// needs reading, and no credential is needed to read one (ADR-0002 §7).
fn gh_available() -> bool {
    let present = Command::new("gh")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !present && std::env::var_os("VIBE_REQUIRE_GH").is_some() {
        panic!(
            "VIBE_REQUIRE_GH is set but `gh` is not on PATH. This control is only \
             meaningful where gh exists, so refusing to run is reported as a \
             failure rather than a skip (ADR-0002 §7)."
        );
    }
    present
}

/// The argv the enum builds for a real create, with `--help` inserted where it
/// cannot be swallowed as a positional.
///
/// Taken from [`GhOp::argv`] rather than retyped. A test that spells the
/// arguments out again is testing its own copy, which is the harness-versus-
/// subject disagreement in its purest form: the day someone edits the enum,
/// this must move with it or fail.
fn probe_argv() -> Vec<String> {
    let op = GhOp::RepoCreate {
        cwd: std::path::PathBuf::from("."),
        name: "vibe-argv-probe-never-created".to_owned(),
        visibility: RepoVisibility::Private,
    };
    let mut argv = op.argv();
    assert_eq!(&argv[..2], ["repo", "create"], "the pair moved: {argv:?}");
    // After the subcommand, before the flags, and *well* before the `--`:
    // appended at the end it would land after the separator and be read as a
    // positional argument, which would turn a help probe into a real create.
    argv.insert(2, "--help".to_owned());
    argv
}

/// Run `gh` with an environment that cannot authenticate, in a directory that
/// is not a repository.
fn run_gh(argv: &[String], cwd: &Path, config_dir: &Path) -> std::process::Output {
    let mut cmd = Command::new("gh");
    cmd.args(argv).current_dir(cwd).env_clear();
    for key in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "PATHEXT",
        "COMSPEC",
    ] {
        if let Some(v) = std::env::var_os(key) {
            cmd.env(key, v);
        }
    }
    // An empty config directory: no host, no token, no alias.
    cmd.env("GH_CONFIG_DIR", config_dir);
    cmd.env("GH_PAGER", "");
    cmd.env("GH_NO_UPDATE_NOTIFIER", "1");
    cmd.env("NO_COLOR", "1");
    cmd.output().expect("gh runs")
}

fn combined(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

/// `cobra`'s vocabulary for "that is not one of my flags".
fn parse_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["unknown flag", "unknown shorthand flag", "unknown command"]
        .iter()
        .any(|m| lower.contains(m))
}

/// **The probe's safety property, asserted where `gh` is not.**
///
/// Everything else in this file skips on a machine without `gh` — including,
/// on the day it matters, the machine where someone edits [`GhOp::argv`]. This
/// does not skip, because it guards the one mistake that would turn a help
/// probe into a live `gh repo create`: `--help` landing *after* the `--`
/// separator, where `cobra` reads it as the repository name rather than as a
/// flag.
///
/// ADR-0002 §7's "fail loudly when the control could not run" applies to the
/// harness's own safety, not only to its assertions.
#[test]
fn the_help_flag_lands_before_the_separator_so_the_probe_cannot_create() {
    let argv = probe_argv();
    let help = argv.iter().position(|a| a == "--help").expect("--help");
    let sep = argv.iter().position(|a| a == "--").expect("separator");
    assert!(
        help < sep,
        "--help fell after the separator, which makes it a positional argument \
         and this probe a real `gh repo create`: {argv:?}"
    );
    // And it is a flag, not the subcommand: `gh --help` prints something else
    // entirely and would satisfy nothing below.
    assert_eq!(&argv[..2], ["repo", "create"], "{argv:?}");
}

/// The parse-error detector, which the paired half of the control below rests
/// on. If it matched nothing, the sabotage assertion would fail loudly; if it
/// matched everything, the real-argv assertion would fail loudly. Both
/// directions are pinned here so a wrong answer arrives as this test rather
/// than as a confusing red in the control.
#[test]
fn the_parse_error_detector_is_sensitive_in_both_directions() {
    assert!(parse_error("unknown flag: --source-tree"));
    assert!(parse_error("Error: unknown shorthand flag: 'z' in -z"));
    assert!(parse_error("unknown command \"crate\" for \"gh repo\""));
    assert!(!parse_error(
        "Usage: gh repo create [<name>] [flags]\n  --source string  ..."
    ));
    assert!(!parse_error(""));
}

#[test]
fn the_argv_the_enum_constructs_is_accepted_by_this_gh() {
    if !gh_available() {
        eprintln!("skipping: gh is not on PATH (this is the CI-verified check)");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("empty-config");
    let work = tmp.path().join("not-a-repo");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let argv = probe_argv();
    eprintln!("probe argv: {argv:?}");
    let out = run_gh(&argv, &work, &cfg);
    let text = combined(&out);
    eprintln!("--- real argv ---\nstatus: {:?}\n{}", out.status, text);

    // Non-vacuity first, and for the same reason ADR-0008 §6's control needs
    // it: a `gh` that failed for an unrelated reason also emits no parse error.
    assert!(
        text.contains("repo create"),
        "gh did not reach its own `repo create` help, so nothing below proves \
         anything:\n{text}"
    );
    assert!(
        !parse_error(&text),
        "gh REJECTED an argument this crate constructs. `GhOp::argv` and this \
         gh release disagree about what `gh repo create` takes, which would \
         make `vibe new --git --private` fail for every user of this release \
         (ADR-0008 §2):\n{text}"
    );
    assert!(
        out.status.success(),
        "the help probe should exit 0:\n{text}"
    );

    // Every flag the enum emits, named in this gh's own help. Redundant with
    // the parse check by design: if a future gh accepted unknown flags
    // silently, this half still notices a rename.
    for flag in ["--source", "--push", "--private", "--public"] {
        assert!(
            text.contains(flag),
            "`{flag}` is not in this gh's `repo create` help:\n{text}"
        );
    }

    // --- the paired half ------------------------------------------------
    //
    // Identical invocation, one element changed to a flag `gh` does not have.
    // Without this, every assertion above would keep passing against a check
    // that had quietly stopped being able to detect anything.
    let mut sabotaged = argv.clone();
    let source = sabotaged
        .iter()
        .position(|a| a == "--source=.")
        .expect("the enum still emits --source=.");
    sabotaged[source] = "--source-tree=.".to_owned();

    let bad = run_gh(&sabotaged, &work, &cfg);
    let bad_text = combined(&bad);
    eprintln!(
        "--- sabotaged argv ---\nstatus: {:?}\n{bad_text}",
        bad.status
    );
    assert!(
        parse_error(&bad_text),
        "gh accepted `--source-tree=.`, a flag it does not have. This control \
         cannot detect a renamed flag, so its green half proves nothing:\n\
         {bad_text}"
    );
}
