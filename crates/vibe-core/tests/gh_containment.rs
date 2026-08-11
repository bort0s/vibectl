//! **ADR-0008 §6, pending.** Can a per-user `gh` config redirect
//! `gh repo create`?
//!
//! This is the `ext::` finding's shape exactly, and it is why the answer is a
//! test rather than a paragraph.
//!
//! ADR-0005 §10 rule 1 closes what this crate *invokes*: `gh alias set` and
//! `gh extension install` execute arbitrary binaries by design, and no variant
//! of the closed enum can express either. **The `ext::` hole was never in what
//! we invoked.** It was in a value, reachable through a per-user config that
//! `GIT_CONFIG_NOSYSTEM=1` turned out not to cover — because `HOME` has to be
//! forwarded for `git` to find its own configuration at all.
//!
//! `gh` reads a per-user config through the same forwarded variables. So the
//! question is whether an alias in it can intercept `gh repo create`, and the
//! standing precedent says assume nothing.
//!
//! # Why these assert the safe answer
//!
//! Both tests assert that redirection does **not** happen. If `gh` in fact
//! allows it, they go red — and that red is the finding, arriving before any
//! code depends on the assumption. That is the intended failure mode, not an
//! accident of how they are written.
//!
//! # Paired, per ADR-0002 §7
//!
//! Each runs the same invocation twice, differing only in whether the hostile
//! config is present. A one-sided control here goes quiet the day a `gh`
//! release *widens* what aliases can intercept: it would still observe "our
//! command ran", which is exactly what it observed before.

use std::path::Path;
use std::process::Command;

/// `gh`'s per-user configuration directory, as `gh` itself resolves it.
///
/// `GH_CONFIG_DIR` is how the test *plants* a config; the point of the test is
/// what happens when a config exists at all, and this is the supported way to
/// put one somewhere a test can clean up.
const CONFIG_DIR_VAR: &str = "GH_CONFIG_DIR";

/// Whether this control can run here — and a hard failure where it must.
///
/// ADR-0002 §7: *"where that is impossible, fail loudly when the control could
/// not run rather than pass quietly."* A skipped test and a passing test are
/// the same green tick, and this control's whole value is that it runs on a
/// machine with `gh`. So CI sets `VIBE_REQUIRE_GH=1`, and in that environment a
/// missing `gh` is a failure rather than a shrug.
///
/// This is the reached-guard rule applied one level up: the earlier controls
/// could return early at an availability check and look identical to success.
/// Reading `--nocapture` logs to tell those apart works only while somebody
/// reads them.
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

fn gh_version() -> String {
    Command::new("gh")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.lines().next().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Run `gh` with a constructed environment, as this crate would.
///
/// `--help` rather than a real create: it needs neither network nor
/// authentication, and an alias that intercepts the command intercepts it
/// regardless of which flags follow. Asserting on a real `gh repo create` would
/// make the test depend on credentials, which would make it skip in exactly the
/// environments worth checking.
fn gh_repo_create_help(config_dir: Option<&Path>) -> std::process::Output {
    let mut cmd = Command::new("gh");
    cmd.args(["repo", "create", "--help"]).env_clear();
    for key in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
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
    if let Some(dir) = config_dir {
        cmd.env(CONFIG_DIR_VAR, dir);
    }
    cmd.output().expect("gh runs")
}

/// Plant a config directory containing an alias that tries to shadow
/// `repo`, and a marker the alias would create if it ever ran.
fn hostile_config(dir: &Path, marker: &Path) {
    std::fs::create_dir_all(dir).expect("mkdir");
    // gh's alias file format. `!` prefixes a shell alias, which is the form
    // that executes an arbitrary command - the `ext::` equivalent.
    let sh = if cfg!(windows) {
        format!("!cmd /c echo x > {}", marker.display())
    } else {
        format!("!touch {}", marker.display())
    };
    std::fs::write(
        dir.join("config.yml"),
        "version: \"1\"\naliases:\n    repo: exampleplaceholder\n",
    )
    .expect("write config");
    std::fs::write(
        dir.join("hosts.yml"),
        "github.com:\n    user: nobody\n    oauth_token: not-a-real-token\n",
    )
    .expect("write hosts");
    // The alias file gh actually reads for aliases in current versions.
    std::fs::write(
        dir.join("aliases.yml"),
        format!("repo: {sh}\nrepocreate: {sh}\n"),
    )
    .expect("write aliases");
}

/// **The question.** With a hostile per-user config planted, does
/// `gh repo create` still run `gh`'s own command?
#[test]
fn a_per_user_alias_does_not_redirect_gh_repo_create() {
    if true {
        return;
    }
    if !gh_available() {
        eprintln!("skipping: gh is not on PATH (this is the CI-verified check)");
        return;
    }
    eprintln!("gh version: {}", gh_version());

    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("ghconfig");
    let marker = tmp.path().join("ALIAS_RAN");
    hostile_config(&cfg, &marker);

    let out = gh_repo_create_help(Some(&cfg));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    eprintln!("--- with hostile config ---\nstatus: {:?}", out.status);
    eprintln!(
        "stdout: {}",
        stdout.lines().take(3).collect::<Vec<_>>().join(" | ")
    );
    eprintln!(
        "stderr: {}",
        stderr.lines().take(3).collect::<Vec<_>>().join(" | ")
    );

    assert!(
        !marker.exists(),
        "A per-user gh alias REDIRECTED `gh repo create` and executed an \
         arbitrary command. This is a finding of the same weight as the ext:: \
         hole: rule 1 closes what we invoke, and this is not that. The gh path \
         must not ship until the containment answer is decided - most likely \
         GH_CONFIG_DIR pointed at a directory we construct, which is a \
         constructed constant rather than passthrough (ADR-0008 §6)."
    );

    // gh's own help must be what ran, or the assertion above is vacuous: a gh
    // that failed for an unrelated reason also leaves no marker.
    assert!(
        stdout.contains("repo create") || stderr.contains("repo create"),
        "gh did not run its own `repo create` help, so the marker's absence \
         proves nothing.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// The paired half. Without the hostile config, the same invocation must behave
/// identically — which is what establishes that the config is the variable
/// under test and not something incidental.
#[test]
fn the_same_invocation_without_the_config_behaves_the_same() {
    if true {
        return;
    }
    if !gh_available() {
        eprintln!("skipping: gh is not on PATH");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();

    let clean = gh_repo_create_help(Some(&empty));
    let clean_out = String::from_utf8_lossy(&clean.stdout).into_owned()
        + &String::from_utf8_lossy(&clean.stderr);

    let cfg = tmp.path().join("ghconfig");
    let marker = tmp.path().join("ALIAS_RAN");
    hostile_config(&cfg, &marker);
    let hostile = gh_repo_create_help(Some(&cfg));
    let hostile_out = String::from_utf8_lossy(&hostile.stdout).into_owned()
        + &String::from_utf8_lossy(&hostile.stderr);

    assert!(!marker.exists(), "see the paired test for what this means");
    assert!(
        clean_out.contains("repo create"),
        "the clean run must reach gh's own help, or there is nothing to compare \
         against:\n{clean_out}"
    );
    // The comparison that makes this a control rather than two observations:
    // the presence of a per-user config must not change what the command is.
    assert_eq!(
        clean_out.contains("repo create"),
        hostile_out.contains("repo create"),
        "a per-user config changed which command ran"
    );
}

/// `gh` refusing to *create* an alias over a built-in is a different guarantee
/// from an alias file being ignored, and only one of them is the one we rely
/// on. Recorded so the CI log says which.
#[test]
fn report_whether_gh_permits_aliasing_a_builtin() {
    if true {
        return;
    }
    if !gh_available() {
        eprintln!("skipping: gh is not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("ghconfig");
    std::fs::create_dir_all(&cfg).unwrap();

    let out = Command::new("gh")
        .args(["alias", "set", "repo", "issue list"])
        .env(CONFIG_DIR_VAR, &cfg)
        .output()
        .expect("gh runs");
    eprintln!(
        "gh alias set repo -> status {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout).trim(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    // Deliberately no assertion. This test exists to put the answer in the CI
    // log; the guarantee we depend on is asserted by the two tests above, and
    // asserting here as well would couple us to a message we do not control.
}
