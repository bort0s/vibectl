//! The six `vibe agents` subcommands, through the binary.
//!
//! The core tests cover policy; these cover the things only the binary can get
//! wrong — the exit-code contract, the stdout/stderr split, and the two rules
//! ADR-0006 puts on the *reporting* rather than on the decision:
//!
//! - a refusal exits `2` (partial), never `1`;
//! - the store-age line appears whenever anything is `NotInStore`, because
//!   "this agent does not exist" and "this machine has not fetched for twelve
//!   days" are different claims and only one of them is about the project.

use std::path::{Path, PathBuf};
use std::process::Command;

fn vibe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vibe"))
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn git(cwd: &Path, args: &[&str]) {
    git_dated(cwd, args, None);
}

/// `date` backdates the commit, which is how a *stale* store is built. Faking
/// it with `--stale-after 0` would not work and should not: the threshold is
/// "older than N days", so on the day of a fetch the store is not stale at any
/// threshold. The age has to be real.
fn git_dated(cwd: &Path, args: &[&str], date: Option<&str>) {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.invalid");
    if let Some(d) = date {
        cmd.env("GIT_AUTHOR_DATE", d).env("GIT_COMMITTER_DATE", d);
    }
    let out = cmd.output().expect("git runs");
    assert!(out.status.success(), "git {args:?} failed");
}

struct Fixture {
    _tmp: tempfile::TempDir,
    store: PathBuf,
    upstream: PathBuf,
    proj: PathBuf,
}

fn fixture(agents: &[&str], declared: &[&str]) -> Fixture {
    fixture_dated(agents, declared, None)
}

fn fixture_dated(agents: &[&str], declared: &[&str], commit_date: Option<&str>) -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let upstream = tmp.path().join("upstream");
    std::fs::create_dir_all(upstream.join("engineering")).unwrap();
    for name in agents {
        std::fs::write(
            upstream.join("engineering").join(format!("{name}.md")),
            format!("---\nname: {name}\ndescription: The {name} agent.\n---\n\nBody.\n"),
        )
        .unwrap();
    }
    git(&upstream, &["init", "-q", "-b", "main"]);
    git(&upstream, &["add", "-A"]);
    git_dated(&upstream, &["commit", "-qm", "agents"], commit_date);

    let proj = tmp.path().join("proj");
    std::fs::create_dir_all(proj.join(".vibe")).unwrap();
    let list = declared
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");
    std::fs::write(
        proj.join(".vibe/project.toml"),
        format!(
            "schema_version = \"1.0\"\n\n[project]\nname = \"proj\"\nstatus = \"active\"\n\n\
             [agents]\ninstalled = [{list}]\n"
        ),
    )
    .unwrap();

    Fixture {
        store: tmp.path().join("store"),
        upstream,
        proj,
        _tmp: tmp,
    }
}

impl Fixture {
    fn cmd(&self, args: &[&str]) -> std::process::Output {
        vibe()
            .arg("agents")
            .args(args)
            .arg("--store-path")
            .arg(&self.store)
            .arg("--store-url")
            .arg(self.upstream.to_string_lossy().replace('\\', "/"))
            .output()
            .expect("run vibe")
    }

    fn update(&self) {
        let out = self.cmd(&["update"]);
        assert!(
            out.status.success(),
            "update failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn update_then_list_shows_the_store_and_marks_what_is_declared() {
    if !git_available() {
        return;
    }
    let f = fixture(&["alpha", "beta"], &["alpha"]);
    f.update();

    let out = f.cmd(&["list", &f.proj.to_string_lossy()]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("alpha"), "{text}");
    assert!(text.contains("beta"), "{text}");
    assert!(text.contains("The alpha agent."), "{text}");
    assert!(
        text.contains("* alpha"),
        "declared agents are marked: {text}"
    );
}

/// `list` before any fetch must not claim there are no agents. There is a
/// difference between an empty store and an empty upstream, and only one of
/// them is a fact about the world.
#[test]
fn list_before_any_update_reports_the_store_not_the_world() {
    let f = fixture(&["alpha"], &[]);
    let out = f.cmd(&["list", &f.proj.to_string_lossy()]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(
        text.contains("store") && text.contains("vibe agents update"),
        "should name the store and the fix, got: {text}"
    );
}

#[test]
fn add_installs_and_is_reported_on_stdout() {
    if !git_available() {
        return;
    }
    let f = fixture(&["alpha"], &[]);
    f.update();

    let out = f.cmd(&["add", "alpha", "--path", &f.proj.to_string_lossy()]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(f.proj.join(".claude/agents/alpha.md").exists());
    assert!(f.proj.join(".vibe/agents.lock").exists());

    let manifest = std::fs::read_to_string(f.proj.join(".vibe/project.toml")).unwrap();
    assert!(manifest.contains(r#"installed = ["alpha"]"#), "{manifest}");
}

#[test]
fn add_dry_run_writes_absolutely_nothing() {
    if !git_available() {
        return;
    }
    let f = fixture(&["alpha"], &[]);
    f.update();
    let before = std::fs::read_to_string(f.proj.join(".vibe/project.toml")).unwrap();

    let out = f.cmd(&[
        "add",
        "alpha",
        "--path",
        &f.proj.to_string_lossy(),
        "--dry-run",
    ]);
    assert!(out.status.success());

    // The whole contract, asserted directly rather than inferred from the
    // absence of an error.
    assert!(!f.proj.join(".claude").exists(), "--dry-run created a file");
    assert!(!f.proj.join(".vibe/agents.lock").exists());
    assert_eq!(
        std::fs::read_to_string(f.proj.join(".vibe/project.toml")).unwrap(),
        before,
        "--dry-run edited the manifest"
    );
    assert!(stderr(&out).contains("dry run"), "{}", stderr(&out));
}

/// The exit-code half of ADR-0006 §6, and the store-age line that makes the
/// report honest.
#[test]
fn a_declared_agent_the_store_lacks_exits_two_and_names_the_stores_age() {
    if !git_available() {
        return;
    }
    // A store whose tip commit is genuinely old, so the age it reports is one
    // it read rather than one the test asserted into existence.
    let f = fixture_dated(
        &["alpha"],
        &["alpha", "engineering-code-reviwer"],
        Some("2020-01-01T00:00:00Z"),
    );
    f.update();

    let out = f.cmd(&["sync", &f.proj.to_string_lossy()]);

    assert_eq!(
        out.status.code(),
        Some(2),
        "a refusal is a partial result, never a failure: {}",
        stderr(&out)
    );
    // The other agent still installed.
    assert!(f.proj.join(".claude/agents/alpha.md").exists());

    let err = stderr(&out);
    assert!(err.contains("engineering-code-reviwer"), "{err}");
    assert!(
        err.contains("vibe agents update"),
        "the store's age must be named alongside the missing agent, or a fact \
         about this machine reads as a claim about the project: {err}"
    );
    // And no near-match is suggested. A plausible guess is worse than an honest
    // empty answer (ADR-0006 trade-off #2).
    assert!(
        !err.contains("engineering-code-reviewer"),
        "suggested a near match: {err}"
    );
}

#[test]
fn status_reports_each_state_and_remove_undeclares() {
    if !git_available() {
        return;
    }
    let f = fixture(&["alpha", "beta"], &[]);
    f.update();
    f.cmd(&["add", "alpha", "beta", "--path", &f.proj.to_string_lossy()]);

    let out = f.cmd(&["status", &f.proj.to_string_lossy()]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("alpha") && text.contains("ok"), "{text}");

    let out = f.cmd(&["remove", "alpha", "--path", &f.proj.to_string_lossy()]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!f.proj.join(".claude/agents/alpha.md").exists());
    assert!(f.proj.join(".claude/agents/beta.md").exists());

    let manifest = std::fs::read_to_string(f.proj.join(".vibe/project.toml")).unwrap();
    assert!(manifest.contains(r#"installed = ["beta"]"#), "{manifest}");
}

/// A URL that would be arbitrary execution is refused before `git` sees it, and
/// the exit code says failure rather than partial — this is not a degraded
/// result, it is a request that will not be carried out.
#[test]
fn a_remote_helper_store_url_is_refused_by_the_binary() {
    let tmp = tempfile::tempdir().unwrap();
    let out = vibe()
        .args(["agents", "update", "--store-url", "ext::sh -c evil"])
        .arg("--store-path")
        .arg(tmp.path().join("store"))
        .output()
        .expect("run vibe");

    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("remote helper"), "{err}");
    assert!(!tmp.path().join("store").exists());
}

/// `--json` goes to stdout and stays parseable; refusals still reach stderr,
/// because a consumer that cannot tell "nothing to do" from "we declined"
/// cannot act on either.
#[test]
fn json_output_is_parseable_and_refusals_still_reach_stderr() {
    if !git_available() {
        return;
    }
    let f = fixture(&["alpha"], &["alpha", "missing-one"]);
    f.update();

    let out = f.cmd(&["sync", &f.proj.to_string_lossy(), "--json", "--dry-run"]);
    let value: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("stdout must be pure JSON");

    assert!(value.get("plan").is_some());
    assert_eq!(
        value["refused"][0]["name"], "missing-one",
        "refusals belong in the payload too"
    );
    assert!(stderr(&out).contains("missing-one"), "{}", stderr(&out));
}
