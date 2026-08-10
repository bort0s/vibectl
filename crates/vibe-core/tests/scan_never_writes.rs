//! **Detection must never write.** `scan` reads; turning its results into
//! manifests is a separate, explicit step.
//!
//! There are two halves to this, and only one of them is a test.
//!
//! The **structural** half is that `Registry::scan` returns a `ScanReport` and
//! there is no code path from it to a `WritePlan`. Since `plan::apply` is the
//! only function in the crate that writes, and it takes a `WritePlan`, scanning
//! cannot write without someone adding a plan constructor to `scan.rs` — a
//! change visible in any diff. That guarantee is enforced by the type system,
//! not by this file. The compile-time check below documents it.
//!
//! The **behavioural** half is here: scan a tree, and prove afterwards that not
//! one byte moved. This catches what the types cannot — a stray `create_dir_all`
//! for a cache directory, a lockfile, a `.gitignore` written "helpfully", or a
//! detector that shells out to a `git` command with a side effect.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use vibe_core::{Config, NoRunner, NullReporter, Registry, ScanRequest};

/// Every path under `root`, with its size and modification time.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (u64, Option<SystemTime>)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path.clone());
                out.insert(path, (0, None));
            } else {
                out.insert(path, (meta.len(), meta.modified().ok()));
            }
        }
    }
    out
}

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, contents).expect("write");
}

fn build_tree(root: &Path) {
    // A node project with an existing manifest, so the manifest-reading path
    // runs too — that is the one place scan opens a file vibe itself owns.
    let a = root.join("alpha");
    write(
        &a,
        "package.json",
        r#"{"name":"alpha","dependencies":{"react":"^19.0.0"}}"#,
    );
    write(&a, ".env.example", "API_KEY=\n");
    write(
        &a,
        ".vibe/project.toml",
        "schema_version = \"1.0\"\n\n[project]\nname = \"alpha\"\nstatus = \"active\"\n",
    );
    write(&a, "node_modules/left-pad/index.js", "module.exports=1\n");

    // A rust project.
    let b = root.join("beta");
    write(&b, "Cargo.toml", "[package]\nname = \"beta\"\n");
    write(&b, "target/debug/junk", "x");

    // A project whose manifest cannot be read — the error path must not try to
    // repair anything.
    let c = root.join("gamma");
    write(&c, "go.mod", "module example.com/gamma\n\ngo 1.23\n");
    write(
        &c,
        ".vibe/project.toml",
        "schema_version = \"9.0\"\n[project]\nname = \"gamma\"\n",
    );

    // A directory that is not a project at all.
    write(root, "notes/README.md", "# just notes\n");
}

#[test]
fn scanning_changes_nothing_on_disk() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    build_tree(root);

    let before = snapshot(root);
    assert!(before.len() > 5, "the fixture should have real content");

    let registry = Registry::open(Config::default()).with_runner(Arc::new(NoRunner));
    let report = registry.scan(&ScanRequest::new(root), &NullReporter);

    // The scan did real work, so this is not vacuously true.
    assert_eq!(report.projects.len(), 3, "alpha, beta, gamma");
    assert_eq!(
        report.unreadable(),
        1,
        "gamma's manifest is from the future"
    );

    let after = snapshot(root);

    // Compare in both directions, so a created file and a deleted one are
    // both caught rather than cancelling out in a length check.
    let created: Vec<_> = after.keys().filter(|k| !before.contains_key(*k)).collect();
    let removed: Vec<_> = before.keys().filter(|k| !after.contains_key(*k)).collect();
    assert!(created.is_empty(), "scan created files: {created:?}");
    assert!(removed.is_empty(), "scan removed files: {removed:?}");

    for (path, meta) in &before {
        assert_eq!(
            after.get(path),
            Some(meta),
            "scan modified {}",
            path.display()
        );
    }
}

#[test]
fn scanning_a_real_git_repository_changes_nothing_either() {
    if std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: git is not on PATH");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let repo = root.join("repo");
    write(&repo, "package.json", r#"{"name":"repo"}"#);

    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@example.invalid"],
        vec!["config", "user.name", "t"],
        // **The fixture must not run anything asynchronously.** `git commit`
        // spawns a *detached* `git maintenance run --auto`, which takes
        // `.git/objects/maintenance.lock` and removes it when it finishes —
        // after `commit` has already returned. The snapshot below then catches
        // the lock in `before`, misses it in `after`, and reports
        // `scan modified …/maintenance.lock` about a file the fixture's own
        // background process deleted. That is a harness side effect wearing the
        // costume of a finding about the subject, and it was an intermittent
        // red on macos-latest — the slowest runner, so the widest window.
        //
        // Disabled at the source rather than filtered out of the snapshot: a
        // lock file appearing under `.git` is precisely what this test exists
        // to notice, so teaching it to ignore lock files would remove the
        // detection along with the noise.
        vec!["config", "gc.auto", "0"],
        vec!["config", "maintenance.auto", "false"],
        vec!["add", "package.json"],
        vec!["commit", "-q", "-m", "initial"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "fixture step `git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // `.git` included: a git subcommand with a side effect — an index refresh,
    // a gc, a lock file — would show up here and nowhere else. This is the
    // reason `git status` was worth being careful about.
    let before = snapshot(root);

    let registry = Registry::open(Config::default());
    let report = registry.scan(&ScanRequest::new(root), &NullReporter);
    assert_eq!(report.projects.len(), 1);
    assert!(
        report.projects[0].detection.last_commit.is_known(),
        "the commit should have been read, or this test proves nothing"
    );

    let after = snapshot(root);
    let created: Vec<_> = after.keys().filter(|k| !before.contains_key(*k)).collect();
    assert!(
        created.is_empty(),
        "scanning a git repository created files: {created:?}"
    );
    for (path, meta) in &before {
        assert_eq!(
            after.get(path),
            Some(meta),
            "scan modified {}",
            path.display()
        );
    }
}

/// The structural half, stated where a reader will find it.
///
/// `scan` hands back a `ScanReport`. If someone later makes it return a
/// `WritePlan`, or adds an `apply` call to `scan.rs`, this assertion is the
/// comment that explains why the reviewer should push back.
#[test]
fn scan_returns_a_report_and_not_a_plan() {
    fn assert_report(_: &vibe_core::ScanReport) {}

    let dir = tempfile::tempdir().expect("tempdir");
    let registry = Registry::open(Config::default()).with_runner(Arc::new(NoRunner));
    let report = registry.scan(&ScanRequest::new(dir.path()), &NullReporter);
    assert_report(&report);

    // `WritePlan` is the only thing `apply` accepts, and nothing in the scan
    // path constructs one. The following would not compile:
    //
    //     let _: vibe_core::WritePlan = registry.scan(&req, &NullReporter);
}
