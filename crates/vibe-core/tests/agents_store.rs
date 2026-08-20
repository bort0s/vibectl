//! The store unit end to end, against a real `git` and a real filesystem.
//!
//! Two things are being tested and they are different in kind.
//!
//! **The containment rules of ADR-0005 §10.** These get negative controls that
//! demonstrate the hole is *real* before asserting we close it. A guard whose
//! threat was never shown to exist is indistinguishable from ceremony, and this
//! project's whole position is that a plausible-looking mitigation is worse
//! than none — that is what the ADR says about the `{git, gh}` allowlist it
//! rejected.
//!
//! **The ownership and ordering rules of ADR-0006 §4–§5.** These are asserted
//! against the state table, on disk, with the two-file write window exercised
//! rather than reasoned about.

use std::path::{Path, PathBuf};
use std::process::Command;

use vibe_core::agents::{AgentState, GitOp, GitUrl, Staleness, StoreConfig, install_path, lock};
use vibe_core::{
    Config, FileOp, NullReporter, ProcessRunner, Registry, SchemaVersion, SystemRunner,
};

const TODAY: &str = "2026-08-10";

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Run `git` in a directory with a constructed environment, for building
/// fixtures. Not the code under test — the code under test is `GitOp`.
///
/// **The exit status is asserted, not returned and ignored.** A fixture step
/// that fails silently does not produce a passing test; it produces a *failing*
/// one that accuses the wrong subsystem. A `git commit` that never ran makes
/// the hooks control below report "the probes never fired", which reads as a
/// finding about hooks and is nothing of the kind. `agents_cli.rs` already
/// asserts this; this helper was the inconsistent one.
fn git(cwd: &Path, args: &[&str]) -> std::process::Output {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "fixture step `git {}` failed in {}: {}{}",
        args.join(" "),
        cwd.display(),
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout),
    );
    out
}

/// A local repository that looks like an agent store.
fn make_upstream(dir: &Path, agents: &[(&str, &str)]) -> PathBuf {
    let repo = dir.join("upstream");
    std::fs::create_dir_all(repo.join("engineering")).unwrap();
    for (name, body) in agents {
        let text =
            format!("---\nname: {name}\ndescription: An agent called {name}.\n---\n\n{body}\n");
        std::fs::write(repo.join("engineering").join(format!("{name}.md")), text).unwrap();
    }
    git(&repo, &["init", "-q", "-b", "main"]);
    // ADR-0002 §7: a fixture must not leave anything running. `git commit`
    // otherwise spawns a detached `git maintenance run --auto`, which is how
    // `scan_never_writes` acquired an intermittent red — and these tests build
    // a repository per case, so it is also gratuitous load.
    git(&repo, &["config", "gc.auto", "0"]);
    git(&repo, &["config", "maintenance.auto", "false"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "agents"]);
    repo
}

fn project(dir: &Path, declared: &[&str]) -> PathBuf {
    let proj = dir.join("proj");
    std::fs::create_dir_all(proj.join(".vibe")).unwrap();
    let installed = declared
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");
    std::fs::write(
        proj.join(".vibe/project.toml"),
        format!(
            "schema_version = \"1.0\"\n\n[project]\nname = \"proj\"\nstatus = \"active\"\n\n\
             # a comment that must survive every write below\n\
             [agents]\ninstalled = [{installed}]\n"
        ),
    )
    .unwrap();
    proj
}

fn registry() -> Registry {
    // No cache path: tests must never touch the developer's real cache.
    Registry::open(Config::discover()).with_cache_path(None)
}

fn store_at(path: &Path, upstream: &Path) -> StoreConfig {
    StoreConfig::default()
        .with_path(path)
        .with_upstream(upstream.to_string_lossy().replace('\\', "/"))
}

// ===================================================================
// Containment: ADR-0005 §10
// ===================================================================

/// **The negative control for the whole `GitUrl` type.**
///
/// It runs `git` the way this crate runs it — `env_clear` plus the rule 3
/// allowlist — and shows that a `ext::` URL *executes an arbitrary command*
/// under exactly those conditions. Without this, `GitUrl::parse`'s `::`
/// rejection is a guard against a hazard nobody demonstrated.
///
/// Two details make it worth a test rather than a comment:
///
/// 1. Modern `git` refuses `ext::` by default, so a naive check looks safe.
/// 2. It is re-enabled by the **per-user** `~/.gitconfig`, and
///    `GIT_CONFIG_NOSYSTEM=1` does *not* suppress that — it covers
///    `/etc/gitconfig` only. `HOME` is on rule 3's allowlist because `git`
///    needs it, so the config is reachable.
///
/// # Why this asserts on a spawn attempt rather than on a created file
///
/// The first version pointed `ext::` at `touch <marker>` and asserted the
/// marker existed. That made the assertion *"`touch` won the race against `git`
/// killing a helper that speaks no protocol"* — and a control whose firing
/// depends on winning a race is not a control. When it stopped firing it would
/// go **green**, silently returning `GitUrl::parse`'s `::` rejection to a guard
/// against a hazard nobody demonstrated: the same outcome as a guard that is
/// never reached, arrived at from the other direction.
///
/// So the assertion moved to something `git` does synchronously and reports
/// itself. Verified on git 2.45.1:
///
/// ```text
/// allowed:  error: cannot spawn vibe-nonexistent-helper-probe: No such file …
///           fatal: Can't run specified command
/// refused:  fatal: transport 'ext' not allowed
/// ```
///
/// Naming a program that cannot exist is the point: `git` reports *what it
/// tried to spawn*, which is the execution primitive itself, and it does so
/// without anything having to run, exist, or win anything. The contrast against
/// the same URL with the config absent is what proves the per-user config is
/// the enabling condition.
#[test]
fn negative_control_a_remote_helper_url_really_does_execute_a_command() {
    if !git_available() {
        return;
    }
    // Cannot exist on any machine, so `git`'s own error names it.
    const PROBE: &str = "vibe-nonexistent-helper-probe";

    let tmp = tempfile::tempdir().unwrap();

    // `git` the way this crate runs it: env_clear plus the rule 3 allowlist.
    let clone_under_rule_3 = |home: &Path, label: &str| -> String {
        let mut cmd = Command::new("git");
        cmd.args(["clone", "--", &format!("ext::{PROBE} --x"), label])
            .current_dir(tmp.path())
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", home)
            // Every hardening variable this crate sets. None of them help.
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C");
        if cfg!(windows) {
            for key in ["SYSTEMROOT", "COMSPEC", "PATHEXT", "USERPROFILE"] {
                if let Some(v) = std::env::var_os(key) {
                    cmd.env(key, v);
                }
            }
        }
        let out = cmd.output().expect("git runs");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        )
    };

    // The hazard: a per-user config re-enables the transport.
    let enabled = tmp.path().join("home-enabled");
    std::fs::create_dir_all(&enabled).unwrap();
    std::fs::write(
        enabled.join(".gitconfig"),
        "[protocol \"ext\"]\n\tallow = always\n",
    )
    .unwrap();

    // The control: byte-identical invocation, no such config.
    let plain = tmp.path().join("home-plain");
    std::fs::create_dir_all(&plain).unwrap();

    let with_config = clone_under_rule_3(&enabled, "victim-enabled");
    let without_config = clone_under_rule_3(&plain, "victim-plain");

    assert!(
        with_config.contains(PROBE),
        "git never tried to spawn the program the URL named, so the ext:: hole \
         did not reproduce on this git version. That is not a reason to relax \
         GitUrl::parse - it is a reason to find out why, because the rejection \
         is cheap and the hole has been real.\n\
         with config: {with_config}\nwithout config: {without_config}"
    );
    assert!(
        !without_config.contains(PROBE),
        "git tried to spawn the URL's program with NO per-user config enabling \
         the transport. That is a wider hole than the one recorded, not a \
         narrower one.\nwithout config: {without_config}"
    );

    // And the guard closes it, at the point where the string is accepted rather
    // than at the point where git runs — the exact URL just shown to be an
    // execution primitive, plus the `touch` form the earlier draft used.
    for refused in [
        format!("ext::{PROBE} --x"),
        "ext::touch /tmp/pwned".to_owned(),
    ] {
        let err = GitUrl::parse(&refused).expect_err("must be refused");
        assert_eq!(err.code(), "VIBE_E_GIT_URL_REJECTED");
    }
}

/// Rule 1 stated as a property of the type rather than of any call site: a
/// hostile string in the only user-controlled slot cannot become an option,
/// because the enum constructs argv and puts `--` in front of it.
#[test]
fn a_hostile_url_cannot_become_an_option_even_if_validation_were_bypassed() {
    let url = GitUrl::parse("https://example.com/x--upload-pack=sh.git").unwrap();
    let argv = GitOp::Clone {
        url,
        dest: PathBuf::from("/d/store"),
    }
    .argv();

    let sep = argv.iter().position(|a| a == "--").expect("`--` present");
    let url_at = argv
        .iter()
        .position(|a| a.contains("upload-pack"))
        .expect("url present");
    assert!(
        sep < url_at,
        "the separator must precede the user-controlled value: {argv:?}"
    );
}

/// Every hook that could plausibly fire on a fetch or a checkout.
const HOOKS_THAT_COULD_FIRE: &[&str] = &[
    "post-update",
    "pre-receive",
    "update",
    "post-receive",
    "post-checkout",
    "post-commit",
    "pre-push",
    "proc-receive",
    "reference-transaction",
    "post-index-change",
];

/// The hooks that have fired so far, by name.
fn fired(markers: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(markers)
        .expect("marker directory")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// **The negative control for the local-path form of rule 4.**
///
/// Rule 4 permits an absolute local path, which means `git clone` gets pointed
/// at a repository this crate did not create. Rule 6 exists because
/// `.git/hooks/post-commit` is execution, so "does cloning run the *source*
/// repository's hooks?" is the next question, and a reader who has just read
/// rule 6 will assume the answer might be yes. ADR-0005 §10 rule 4 says it is
/// no. This checks that rather than citing it, against whatever `git` the
/// machine has — which on `ubuntu-latest` is the version the `ext::` hole was
/// found on, so CI is where the claim gets confirmed rather than asserted.
///
/// **The positive control is the point.** Probes that never fire would make the
/// negative half pass vacuously, and a clone that failed would leave exactly
/// the same silence as a clone that ran no hooks. Both are the ADR-0002 §7
/// failure — a guard that was never reached — so the probes are proved live and
/// the clone is proved to have succeeded *before* silence is read as evidence.
#[test]
fn negative_control_cloning_a_local_repo_does_not_run_the_source_repos_hooks() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let markers = tmp.path().join("markers");
    std::fs::create_dir_all(&markers).unwrap();

    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    git(&src, &["init", "-q", "-b", "main"]);

    // Forward slashes: this string goes into a shell script, and git-for-
    // windows runs hooks through its own `sh`, which does not want backslashes.
    let marker_dir = markers.to_string_lossy().replace('\\', "/");
    for &hook in HOOKS_THAT_COULD_FIRE {
        let script = format!("#!/bin/sh\ntouch \"{marker_dir}/{hook}\"\n");
        let path = src.join(".git").join("hooks").join(hook);
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    // --- positive control: the probes are live ---------------------------
    std::fs::write(src.join("a.txt"), "hi\n").unwrap();
    git(&src, &["add", "-A"]);
    git(&src, &["commit", "-qm", "seed"]);
    let after_commit = fired(&markers);
    assert!(
        after_commit.iter().any(|h| h == "post-commit"),
        "the hook probes never fired on an ordinary commit, so the silence \
         after the clone below would prove nothing about cloning. Fix the \
         probes rather than trusting the result: {after_commit:?}"
    );

    // --- the negative result ---------------------------------------------
    for entry in std::fs::read_dir(&markers).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }

    let dest = tmp.path().join("clone");
    let url = GitUrl::parse(&src.to_string_lossy().replace('\\', "/"))
        .expect("an absolute local path is a rule 4 URL");
    let out = SystemRunner::default()
        .run_git_op(&GitOp::Clone {
            url,
            dest: dest.clone(),
        })
        .expect("the clone runs");

    // Asserted, never assumed. A clone that failed leaves no markers and looks
    // identical to the result being claimed.
    assert!(out.success(), "the clone itself failed: {out:?}");
    assert!(
        dest.join("a.txt").is_file(),
        "the clone produced no working tree, so nothing was exercised: {out:?}"
    );

    let after_clone = fired(&markers);
    assert!(
        after_clone.is_empty(),
        "cloning a local repository executed that repository's hooks: \
         {after_clone:?}. ADR-0005 §10 rule 4 records that it does not, and \
         rule 6 exists because a hook in a repository we did not write is \
         execution. If this fails, the ADR is wrong, not this test."
    );
}

/// ADR-0006 §7 at the command where it was missing.
///
/// `list` reads the store, so it must be able to say how old the store is.
/// Every name it prints is a fact about *this machine's copy*, and a reader
/// with no age cannot tell a complete list from a twelve-day-old one — the same
/// substitution as reporting "this agent does not exist" when the truth is
/// "this machine has not fetched since Tuesday".
///
/// **Paired, per ADR-0002 §7**: a stale store reports stale *and* a fresh one
/// reports fresh. Asserting only the stale direction would pass equally well
/// against a `staleness` field wired to a constant, which is a control that
/// cannot fail in the direction that matters.
///
/// Deterministic, also per §7: the store's committer date is pinned, so the
/// only thing that moves between the two halves is the date being asked about.
/// Nothing here depends on when the test ran or how long it took.
#[test]
fn list_reports_the_store_age_and_does_so_in_both_directions() {
    if !git_available() {
        return;
    }
    const COMMITTED: &str = "2026-08-01T12:00:00+00:00";

    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);

    // Pin the committer date, which is what `staleness` reads (`log -1
    // --format=%cI`). Without this the store's age is "however long ago the
    // fixture ran", and the stale half of this test could never be written.
    let out = Command::new("git")
        .args(["commit", "--amend", "--no-edit", "-q"])
        .current_dir(&upstream)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
        .env("GIT_AUTHOR_DATE", COMMITTED)
        .env("GIT_COMMITTER_DATE", COMMITTED)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "could not pin the fixture's commit date: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let store = store_at(&tmp.path().join("store"), &upstream);
    let reg = registry();
    reg.agents_update_store(&store).expect("clone");

    // Three days on: inside the seven-day default, so nothing is said.
    let fresh = reg.agents_list(None, &store, "2026-08-04").expect("list");
    assert_eq!(
        fresh.staleness,
        Staleness::Days {
            days: 3,
            stale: false
        }
    );
    assert!(
        !fresh.staleness.worth_reporting(),
        "a store fetched three days ago must not nag"
    );

    // Twenty days on: the same store, the same clone, only the observer's date
    // has moved.
    let stale = reg.agents_list(None, &store, "2026-08-21").expect("list");
    assert_eq!(
        stale.staleness,
        Staleness::Days {
            days: 20,
            stale: true
        }
    );
    assert!(stale.staleness.worth_reporting());

    // The age travels *beside* the agents, not instead of them: both calls
    // return the identical listing.
    assert_eq!(fresh.listings, stale.listings);
    assert_eq!(fresh.listings.len(), 1);
    assert_eq!(fresh.listings[0].name, "a");
}

/// The store proves it is the store before `reset --hard` gets near it.
///
/// This is the guard that stops a mistyped `--store-path` from destroying the
/// user's real work, and it is the only place in the crate where a destructive
/// git operation runs outside a `WritePlan`.
#[test]
fn update_refuses_a_directory_that_is_not_our_store() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);

    // A repository the user cares about, sitting where the store was pointed —
    // and, crucially, one with a *working* `origin` of its own. Without that
    // the fetch would fail on its own and the test would pass for the wrong
    // reason, proving nothing about the guard. With it, everything downstream
    // of the check succeeds, so the only thing standing between `reset --hard`
    // and the user's uncommitted work is the check itself.
    let victims_origin = tmp.path().join("their-real-origin");
    std::fs::create_dir_all(&victims_origin).unwrap();
    std::fs::write(victims_origin.join("thesis.txt"), "the committed draft").unwrap();
    git(&victims_origin, &["init", "-q", "-b", "main"]);
    git(&victims_origin, &["add", "-A"]);
    git(&victims_origin, &["commit", "-qm", "draft"]);

    let victims_repo = tmp.path().join("my-real-work");
    let out = git(
        tmp.path(),
        &[
            "clone",
            "-q",
            &victims_origin.to_string_lossy(),
            &victims_repo.to_string_lossy(),
        ],
    );
    assert!(out.status.success(), "fixture clone failed");

    // Uncommitted work, which is exactly what `reset --hard` destroys.
    std::fs::write(victims_repo.join("thesis.txt"), "eight months plus today").unwrap();

    let store = store_at(&victims_repo, &upstream);
    let err = registry()
        .agents_update_store(&store)
        .expect_err("must refuse");

    assert_eq!(err.code(), "VIBE_E_STORE_NOT_OURS");
    assert_eq!(
        std::fs::read_to_string(victims_repo.join("thesis.txt")).unwrap(),
        "eight months plus today",
        "the uncommitted change must still be there - if this line fails, the \
         ownership check is what was holding it up"
    );
}

#[test]
fn update_refuses_a_non_empty_directory_that_is_not_a_repository_at_all() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);
    let occupied = tmp.path().join("occupied");
    std::fs::create_dir_all(&occupied).unwrap();
    std::fs::write(occupied.join("notes.md"), "mine").unwrap();

    let err = registry()
        .agents_update_store(&store_at(&occupied, &upstream))
        .expect_err("must refuse");
    assert_eq!(err.code(), "VIBE_E_STORE_NOT_A_REPOSITORY");
    assert!(occupied.join("notes.md").exists());
}

// ===================================================================
// The store, and the six commands
// ===================================================================

#[test]
fn update_clones_then_fast_forwards() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a"), ("b", "body b")]);
    let store = store_at(&tmp.path().join("store"), &upstream);
    let reg = registry();

    let first = reg.agents_update_store(&store).expect("clone");
    assert!(first.cloned);
    assert_eq!(first.agents, 2, "both agents were read from frontmatter");

    // A second update with nothing upstream changes nothing.
    let second = reg.agents_update_store(&store).expect("fetch");
    assert!(!second.cloned);
    assert!(!second.changed(), "nothing moved upstream");

    // Now move upstream and fast-forward.
    std::fs::write(
        upstream.join("engineering/a.md"),
        "---\nname: a\ndescription: An agent called a.\n---\n\nbody a, revised\n",
    )
    .unwrap();
    git(&upstream, &["commit", "-aqm", "revise a"]);

    let third = reg.agents_update_store(&store).expect("fetch");
    assert!(third.changed());
    assert_ne!(third.from_rev, third.to_rev);

    let catalogue = reg.agents_list(None, &store, TODAY).expect("list");
    let listings = &catalogue.listings;
    assert_eq!(listings.len(), 2);
    assert_eq!(listings[0].name, "a");
    assert_eq!(
        listings[0].description.as_deref(),
        Some("An agent called a.")
    );
}

/// `add` writes the file, the lockfile, and the manifest — in that order, and
/// bumps the schema version because `[agents]` is a 1.1 feature.
#[test]
fn add_installs_declares_and_migrates_the_schema() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);
    let store = store_at(&tmp.path().join("store"), &upstream);
    let proj = project(tmp.path(), &[]);
    let reg = registry();
    reg.agents_update_store(&store).unwrap();

    let planned = reg
        .plan_agents_add(&proj, &["a".to_owned()], false, &store, TODAY)
        .expect("plan");
    assert!(planned.refused.is_empty(), "{:?}", planned.refused);

    // ADR-0006 §4: the agent file lands before the lockfile, so a crash between
    // them leaves an unowned file rather than a claim of ownership over a file
    // we never wrote.
    let paths: Vec<String> = planned
        .plan
        .ops
        .iter()
        .map(|op| {
            op.resolved_path(&planned.plan.root)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    let agent_at = paths.iter().position(|p| p.ends_with("agents/a.md"));
    let lock_at = paths.iter().position(|p| p.ends_with("agents.lock"));
    assert!(
        agent_at < lock_at,
        "the agent file must be written before the lockfile: {paths:?}"
    );

    reg.apply(&planned.plan, &NullReporter).expect("apply");

    // The file is the store's bytes, verbatim. No marker, no templating — an
    // installed agent is a user-editable artifact, not a rendered file.
    let installed = proj.join(install_path("a"));
    let store_copy = std::fs::read(tmp.path().join("store/engineering/a.md")).expect("store copy");
    assert_eq!(std::fs::read(&installed).unwrap(), store_copy);

    // Declared, and the version migrated with it.
    let text = std::fs::read_to_string(proj.join(".vibe/project.toml")).unwrap();
    assert!(text.contains(r#"installed = ["a"]"#), "{text}");
    assert!(
        text.contains(&format!(r#"schema_version = "{}""#, SchemaVersion::CURRENT)),
        "{text}"
    );
    assert!(
        text.contains("# a comment that must survive every write below"),
        "the manifest write disturbed something it did not address:\n{text}"
    );

    // And the state table agrees.
    let status = reg.agents_status(&proj, &store, TODAY).unwrap();
    assert_eq!(status.report.agents[0].state, AgentState::Installed);
}

/// The rule this feature is most likely to break: editing an installed agent is
/// the *normal* reason to have installed one.
#[test]
fn an_edited_agent_is_never_overwritten_without_force() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);
    let store = store_at(&tmp.path().join("store"), &upstream);
    let proj = project(tmp.path(), &["a"]);
    let reg = registry();
    reg.agents_update_store(&store).unwrap();

    let planned = reg
        .plan_agents_sync(&proj, false, false, &store, TODAY)
        .unwrap();
    reg.apply(&planned.plan, &NullReporter).unwrap();

    let installed = proj.join(install_path("a"));
    std::fs::write(&installed, "I rewrote this by hand and I meant it").unwrap();

    // Upstream moves.
    std::fs::write(
        upstream.join("engineering/a.md"),
        "---\nname: a\ndescription: An agent called a.\n---\n\nupstream's new body\n",
    )
    .unwrap();
    git(&upstream, &["commit", "-aqm", "revise"]);
    reg.agents_update_store(&store).unwrap();

    let status = reg.agents_status(&proj, &store, TODAY).unwrap();
    assert_eq!(status.report.agents[0].state, AgentState::Modified);

    // Refused, and the plan is empty rather than the edit being lost.
    let refused = reg
        .plan_agents_sync(&proj, true, false, &store, TODAY)
        .unwrap();
    assert!(refused.plan.is_empty(), "{:?}", refused.plan.ops);
    assert_eq!(refused.refused.len(), 1);
    assert!(refused.partial(), "a refusal is a partial result");
    assert_eq!(
        std::fs::read_to_string(&installed).unwrap(),
        "I rewrote this by hand and I meant it"
    );

    // `--force` is the user saying so out loud.
    let forced = reg
        .plan_agents_sync(&proj, true, true, &store, TODAY)
        .unwrap();
    reg.apply(&forced.plan, &NullReporter).unwrap();
    assert!(
        std::fs::read_to_string(&installed)
            .unwrap()
            .contains("upstream's new body")
    );
}

/// A file we did not install is never adopted, and `add` says so rather than
/// taking ownership quietly.
#[test]
fn add_refuses_to_adopt_a_file_it_did_not_install() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);
    let store = store_at(&tmp.path().join("store"), &upstream);
    let proj = project(tmp.path(), &["a"]);
    registry().agents_update_store(&store).unwrap();

    // Somebody else's file, at the path ours would take.
    let path = proj.join(install_path("a"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "hand-written, or another tool's").unwrap();

    let planned = registry()
        .plan_agents_add(&proj, &["a".to_owned()], false, &store, TODAY)
        .unwrap();

    assert_eq!(planned.refused.len(), 1);
    assert_eq!(planned.refused[0].state, AgentState::PresentUnowned);
    // Not even --force adopts it: force overrides *our* caution about *our*
    // file, not the ownership rule.
    let forced = registry()
        .plan_agents_add(&proj, &["a".to_owned()], true, &store, TODAY)
        .unwrap();
    assert_eq!(forced.refused.len(), 1);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "hand-written, or another tool's"
    );
}

/// `remove` inverts the write order, and only ever removes what is ours.
#[test]
fn remove_drops_the_lock_entry_before_the_file() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a"), ("b", "body b")]);
    let store = store_at(&tmp.path().join("store"), &upstream);
    let proj = project(tmp.path(), &[]);
    let reg = registry();
    reg.agents_update_store(&store).unwrap();

    let add = reg
        .plan_agents_add(
            &proj,
            &["a".to_owned(), "b".to_owned()],
            false,
            &store,
            TODAY,
        )
        .unwrap();
    reg.apply(&add.plan, &NullReporter).unwrap();

    let planned = reg
        .plan_agents_remove(&proj, &["a".to_owned()], &store, TODAY)
        .unwrap();

    // ADR-0006 §4, inverted: a crash leaves a file with no entry (unowned,
    // untouched, reported) and never an entry pointing at something gone.
    let lock_at = planned.plan.ops.iter().position(|op| {
        op.resolved_path(&planned.plan.root)
            .ends_with("agents.lock")
    });
    let del_at = planned
        .plan
        .ops
        .iter()
        .position(|op| matches!(op, FileOp::RemoveOwnedAgent { .. }));
    assert!(
        lock_at < del_at,
        "the lock entry must be dropped first: {:?}",
        planned.plan.ops
    );

    reg.apply(&planned.plan, &NullReporter).unwrap();
    assert!(!proj.join(install_path("a")).exists(), "a should be gone");
    assert!(proj.join(install_path("b")).exists(), "b should remain");

    let text = std::fs::read_to_string(proj.join(".vibe/project.toml")).unwrap();
    assert!(text.contains(r#"installed = ["b"]"#), "{text}");
}

#[test]
fn remove_refuses_a_file_vibe_did_not_install() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);
    let store = store_at(&tmp.path().join("store"), &upstream);
    let proj = project(tmp.path(), &["a"]);
    registry().agents_update_store(&store).unwrap();

    let path = proj.join(install_path("a"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "not ours").unwrap();

    let planned = registry()
        .plan_agents_remove(&proj, &["a".to_owned()], &store, TODAY)
        .unwrap();
    assert_eq!(planned.refused.len(), 1);
    assert!(planned.plan.is_empty());
    assert!(path.exists(), "not ours to delete");
}

/// ADR-0006 §4's recovery path, exercised rather than reasoned about: the state
/// after a crash between the two writes, and the claim that re-running the same
/// command fixes it.
#[test]
fn a_crash_between_the_two_writes_leaves_an_unowned_file_that_add_recovers() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);
    let store = store_at(&tmp.path().join("store"), &upstream);
    let proj = project(tmp.path(), &["a"]);
    let reg = registry();
    reg.agents_update_store(&store).unwrap();

    // Simulate the crash by applying only the ops before the lockfile write —
    // which is exactly what "the agent file lands first" means.
    let planned = reg
        .plan_agents_add(&proj, &["a".to_owned()], false, &store, TODAY)
        .unwrap();
    let lock_at = planned
        .plan
        .ops
        .iter()
        .position(|op| {
            op.resolved_path(&planned.plan.root)
                .ends_with("agents.lock")
        })
        .expect("a lockfile write");
    let partial = vibe_core::WritePlan::new(
        vibe_core::PlanIntent::AgentsAdd,
        proj.clone(),
        planned.plan.ops[..lock_at].to_vec(),
    );
    reg.apply(&partial, &NullReporter).unwrap();

    assert!(proj.join(install_path("a")).exists());
    assert!(!lock::lock_path(&proj).exists(), "no lockfile yet");

    // The state the table names for it.
    let status = reg.agents_status(&proj, &store, TODAY).unwrap();
    assert_eq!(
        status.report.agents[0].state,
        AgentState::PresentUnowned,
        "a file with no lock entry is unowned - that is the recoverable half"
    );

    // §4's recovery path: a bare re-run of `add` fixes it. This works *because*
    // the bytes are identical to the store's, which is the one condition under
    // which adopting an unowned file changes nothing on disk. It is not a
    // relaxation of the ownership rule — a differing file is still refused, as
    // the next assertion shows.
    let again = reg
        .plan_agents_add(&proj, &["a".to_owned()], false, &store, TODAY)
        .unwrap();
    assert!(
        again.refused.is_empty(),
        "a bare re-run must recover, or §4's write ordering loses its \
         justification: {:?}",
        again.refused
    );
    reg.apply(&again.plan, &NullReporter).unwrap();

    let recovered = reg.agents_status(&proj, &store, TODAY).unwrap();
    assert_eq!(recovered.report.agents[0].state, AgentState::Installed);

    // And the exception is exactly as narrow as it claims. A file with
    // different content at the same path is still someone else's.
    let other = project(&tmp.path().join("second"), &["a"]);
    std::fs::create_dir_all(other.join(".claude/agents")).unwrap();
    std::fs::write(other.join(install_path("a")), "somebody else's work").unwrap();
    let refused = reg
        .plan_agents_add(&other, &["a".to_owned()], false, &store, TODAY)
        .unwrap();
    assert_eq!(refused.refused.len(), 1);
    assert_eq!(refused.refused[0].state, AgentState::PresentUnowned);
}

/// A declared agent the store lacks never fails the command, and the store's
/// age is what makes the report honest.
#[test]
fn sync_installs_what_it_can_and_reports_what_it_cannot() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);
    let store = store_at(&tmp.path().join("store"), &upstream);
    let proj = project(tmp.path(), &["a", "engineering-code-reviwer"]);
    let reg = registry();
    reg.agents_update_store(&store).unwrap();

    let planned = reg
        .plan_agents_sync(&proj, false, false, &store, TODAY)
        .unwrap();

    assert_eq!(planned.refused.len(), 1);
    assert_eq!(planned.refused[0].name, "engineering-code-reviwer");
    assert_eq!(planned.refused[0].state, AgentState::NotInStore);
    assert!(planned.partial(), "partial, not a failure");
    assert!(!planned.plan.is_empty(), "the other agent still installs");

    reg.apply(&planned.plan, &NullReporter).unwrap();
    assert!(proj.join(install_path("a")).exists());

    // Nothing suggests a near match. A typo and a genuinely absent agent are
    // reported identically, on purpose (ADR-0006 trade-off #2).
    assert!(
        !planned.refused[0].why.contains("code-reviewer"),
        "a suggested name is a plausible guess dressed as help"
    );

    // `sync` never edits the manifest: declaring is the user's act.
    let text = std::fs::read_to_string(proj.join(".vibe/project.toml")).unwrap();
    assert!(text.contains("engineering-code-reviwer"), "{text}");
}

/// A corrupt lockfile stops every write, and does not silently disown anything.
#[test]
fn a_corrupt_lockfile_refuses_writes_rather_than_treating_agents_as_unowned() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);
    let store = store_at(&tmp.path().join("store"), &upstream);
    let proj = project(tmp.path(), &["a"]);
    let reg = registry();
    reg.agents_update_store(&store).unwrap();

    let planned = reg
        .plan_agents_sync(&proj, false, false, &store, TODAY)
        .unwrap();
    reg.apply(&planned.plan, &NullReporter).unwrap();

    std::fs::write(lock::lock_path(&proj), "this is not toml {{{").unwrap();

    // Reads still work and say why.
    let status = reg.agents_status(&proj, &store, TODAY).unwrap();
    assert!(!status.report.ownership_known);
    assert_eq!(status.report.agents[0].state, AgentState::Unverifiable);
    assert!(status.report.lock_note.is_some());

    // Writes refuse.
    for err in [
        reg.plan_agents_add(&proj, &["a".to_owned()], true, &store, TODAY)
            .err(),
        reg.plan_agents_remove(&proj, &["a".to_owned()], &store, TODAY)
            .err(),
        reg.plan_agents_sync(&proj, true, true, &store, TODAY).err(),
    ] {
        assert_eq!(
            err.expect("must refuse").code(),
            "VIBE_E_OWNERSHIP_UNKNOWN",
            "--force must not override a lockfile we cannot read: force is about \
             our own file, not about whether the file is ours"
        );
    }

    // And the installed agent is still on disk, untouched.
    assert!(proj.join(install_path("a")).exists());
}

/// Nothing in this crate reaches the network except `update`.
#[test]
fn every_command_except_update_works_with_the_network_gone() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);
    let store_dir = tmp.path().join("store");
    let store = store_at(&store_dir, &upstream);
    let proj = project(tmp.path(), &["a"]);
    let reg = registry();
    reg.agents_update_store(&store).unwrap();

    // Delete the upstream entirely. Anything that reaches for it now fails.
    std::fs::remove_dir_all(&upstream).unwrap();

    assert!(reg.agents_list(Some(&proj), &store, TODAY).is_ok());
    assert!(reg.agents_status(&proj, &store, TODAY).is_ok());
    let planned = reg
        .plan_agents_sync(&proj, false, false, &store, TODAY)
        .expect("sync is offline");
    reg.apply(&planned.plan, &NullReporter).unwrap();
    assert!(proj.join(install_path("a")).exists());
    assert!(
        reg.plan_agents_remove(&proj, &["a".to_owned()], &store, TODAY)
            .is_ok()
    );
}

/// **THE INVARIANT: no plan is ever produced from a read that failed.**
///
/// Written on the invariant rather than on the case, because the previous repair
/// of this exact shape was written on a case. Round 8 established *absent is not
/// unreadable* for `read_document` — right argument, control built — and the
/// identical defect sat two functions away in the read-error form rather than
/// the is-a-directory form. A control on the shape covers the shape.
///
/// # The chain this closes, measured before it was repaired
///
/// 1. the `AgentState` scan read with `.ok()`, so a read **error** became
///    `AgentState::Missing` — a fact about the world inferred from a failure to
///    look;
/// 2. `Missing` does not need `--force`, so `install`'s overwrite gate let it
///    through where `Modified` refuses;
/// 3. `install` read again with `.ok()`, got `None`, and planned a
///    **`CreateFile`** carrying the store's contents, aimed at a path that
///    exists.
///
/// **A fourth link was claimed here and there is no fourth link.** *Struck
/// 2026-08-20.* This doc said *"`apply` runs `CreateFile` and `UpdateFile`
/// through one arm, so it replaces"*, and concluded *"a user's edited agent,
/// replaced"*. **Measured: `apply` refuses** — `check_precondition` runs over
/// every op before any op runs and returns `TargetExists`. Nothing is written,
/// on any path. The struck sentence described the *write* arm and omitted the
/// *gate* in front of it; it was inferred from a summary rather than run. See
/// ADR-0001 §3b, where the withdrawal and its cause are recorded.
///
/// So what the chain costs is a wrong `status` label, a dry run that misreports,
/// and an error naming a symptom whose cause was discarded three steps earlier.
/// Each link was separately defensible and the composition was not — which is
/// why the assertion below is on the **plan**, the place they meet. **The
/// assertion is unchanged by the withdrawal**, and that is the point of putting
/// it on the plan: *no plan is ever produced from a failed read* is worth
/// holding whether or not a downstream gate happens to catch the plan today.
/// The gate is one line in another crate and this control does not depend on it.
///
/// # The failure mode this can construct, and the one it cannot
///
/// A **directory where the file belongs** is a deterministic read failure that
/// is not `NotFound`, on every platform. It is the round-8 shape in its
/// read-error form.
///
/// A transient `PermissionDenied` — the one ADR-0001 §3a measures during a
/// replacement — needs a second process holding the file, and is **not**
/// constructed here. The invariant is the same for both, which is the argument
/// for asserting the invariant: this control does not have to enumerate the ways
/// a read can fail, only that a failed one produces nothing.
#[test]
fn no_plan_is_ever_produced_from_a_failed_read() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let upstream = make_upstream(tmp.path(), &[("a", "body a")]);
    let store = store_at(&tmp.path().join("store"), &upstream);
    let proj = project(tmp.path(), &["a"]);
    registry().agents_update_store(&store).unwrap();

    let planned = registry()
        .plan_agents_add(&proj, &["a".to_owned()], false, &store, TODAY)
        .unwrap();
    registry().apply(&planned.plan, &NullReporter).unwrap();

    let path = proj.join(install_path("a"));
    std::fs::write(&path, "the user's own edits").unwrap();

    // Premise one: this is the protected state. Without it the assertion below
    // is about an agent nobody was refusing to overwrite anyway.
    let status = registry().agents_status(&proj, &store, TODAY).unwrap();
    assert_eq!(
        status
            .report
            .agents
            .iter()
            .find(|a| a.name == "a")
            .unwrap()
            .state,
        AgentState::Modified
    );

    // Premise two, and it is the pairing: with the read WORKING, a plan is
    // produced — so "no ops" below is not satisfied by a build that never plans.
    let forced = registry()
        .plan_agents_add(&proj, &["a".to_owned()], true, &store, TODAY)
        .unwrap();
    assert!(
        !forced.plan.ops.is_empty(),
        "the fixture cannot plan at all, so an empty plan proves nothing"
    );

    // Break the READ, not the file's existence.
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();

    for force in [false, true] {
        let planned = registry()
            .plan_agents_add(&proj, &["a".to_owned()], force, &store, TODAY)
            .unwrap();

        assert!(
            planned.plan.ops.is_empty(),
            "a failed read produced {} op(s) with force={force}: {:?}. Each link \
             in this chain was separately defensible; the composition replaces a \
             file the tool could not read.",
            planned.plan.ops.len(),
            planned.plan.ops
        );
        assert_eq!(
            planned.refused.len(),
            1,
            "a failed read must be REPORTED, not silently skipped — silence here \
             is the same absence a successful no-op produces"
        );
        assert_eq!(planned.refused[0].state, AgentState::Unverifiable);
    }

    // And the state itself says "cannot tell" rather than "gone".
    let status = registry().agents_status(&proj, &store, TODAY).unwrap();
    assert_eq!(
        status
            .report
            .agents
            .iter()
            .find(|a| a.name == "a")
            .unwrap()
            .state,
        AgentState::Unverifiable,
        "a file that could not be read must not report as Missing — that is a \
         fact about the world inferred from a failure to look"
    );
}
