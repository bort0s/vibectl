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

use vibe_core::agents::{AgentState, GitOp, GitUrl, StoreConfig, install_path, lock};
use vibe_core::{Config, FileOp, NullReporter, Registry, SchemaVersion};

const TODAY: &str = "2026-08-10";

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Run `git` in a directory with a constructed environment, for building
/// fixtures. Not the code under test — the code under test is `GitOp`.
fn git(cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
        .output()
        .expect("git runs")
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
#[test]
fn negative_control_a_remote_helper_url_really_does_execute_a_command() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join(".gitconfig"),
        "[protocol \"ext\"]\n\tallow = always\n",
    )
    .unwrap();

    let marker = tmp.path().join("EXECUTED");
    // A program that creates a file, named without a shell: git splits the
    // `ext::` command on whitespace without shell quoting, so a `sh -c "…"`
    // payload would be mangled and prove nothing. `touch` is what git-for-
    // windows ships in its POSIX toolchain, so it resolves on both platforms.
    let payload = format!("ext::touch {}", marker.display());

    let mut cmd = Command::new("git");
    cmd.args(["clone", "--", &payload, "victim"])
        .current_dir(tmp.path())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", &home)
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
    let _ = cmd.output().expect("git runs");

    assert!(
        marker.exists(),
        "the ext:: hole did not reproduce on this machine/git version. That is \
         not a reason to relax GitUrl::parse - it is a reason to check why, \
         because the rejection is cheap and the hole has been real."
    );

    // And the guard closes it, at the point where the string is accepted rather
    // than at the point where git runs.
    let err = GitUrl::parse(&payload).expect_err("must be refused");
    assert_eq!(err.code(), "VIBE_E_GIT_URL_REJECTED");
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

    let listings = reg.agents_list(None, &store).expect("list");
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
        .map(|op| op.path().to_string_lossy().replace('\\', "/"))
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
    let lock_at = planned
        .plan
        .ops
        .iter()
        .position(|op| op.path().ends_with("agents.lock"));
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
        .position(|op| op.path().ends_with("agents.lock"))
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

    assert!(reg.agents_list(Some(&proj), &store).is_ok());
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
