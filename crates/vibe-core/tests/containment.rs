//! The rules that make "never destructive" and "never writes outside the
//! project" checkable rather than intended (ADR-0005 §10).
//!
//! Every guard here is tested against a violation, not just against the happy
//! path. A containment check that has never rejected anything is decoration.

use std::path::PathBuf;

use vibe_core::plan::{self, SkipReason, validate_path};
use vibe_core::{
    ApplyOutcome, CollectingReporter, Config, CoreError, Event, FileOp, NewRequest, NullReporter,
    PlanIntent, Registry, Reporter, WritePlan,
};

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// Canonicalised, because on macOS `/var` is a symlink to `/private/var` and
/// the containment check compares canonical forms.
fn root_of(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().canonicalize().expect("canonicalize tempdir")
}

// --- rule 5: nothing may land under .git/ -------------------------------

#[test]
fn a_write_into_a_git_directory_is_refused() {
    let dir = tmp();
    let root = root_of(&dir);

    // The concrete attack: an additive file write that becomes execution the
    // next time any git command runs in this repository.
    let hook = root.join(".git").join("hooks").join("post-commit");
    let err = validate_path(&hook, &root).expect_err("writes under .git must be refused");
    assert!(matches!(err, CoreError::PathInsideGitDir { .. }));
    assert_eq!(err.code(), "VIBE_E_PATH_INSIDE_GIT_DIR");
}

#[test]
fn git_rejection_covers_config_worktrees_and_odd_casing() {
    let dir = tmp();
    let root = root_of(&dir);

    for suffix in [
        ".git/config",
        ".git",
        ".git/hooks/pre-push",
        // A worktree's real git dir is still addressed through a `.git`
        // component, so the same rule covers it.
        ".git/worktrees/wt/hooks/post-checkout",
        // Windows and macOS default filesystems are case-insensitive, so a
        // case-sensitive check would be bypassable there.
        ".GIT/hooks/post-commit",
        ".Git/config",
    ] {
        let p = root.join(suffix.replace('/', std::path::MAIN_SEPARATOR_STR));
        match validate_path(&p, &root) {
            Err(CoreError::PathInsideGitDir { .. }) => {}
            Err(other) => panic!("{suffix} was rejected, but for the wrong reason: {other}"),
            Ok(()) => panic!("{suffix} should have been rejected and was not"),
        }
    }
}

/// The rejection above is what bounds the TOCTOU residual risk documented in
/// ADR-0005 §10. Without it, "the worst case is an unintended additive write"
/// is false, because an additive write to a hook is code execution.
#[test]
fn a_normal_project_file_is_allowed() {
    let dir = tmp();
    let root = root_of(&dir);
    validate_path(&root.join(".vibe").join("project.toml"), &root).expect("the normal case");
    validate_path(&root.join("README.md"), &root).expect("render targets");
    // A directory merely *named* like a dotfile is fine; only `.git` is special.
    validate_path(&root.join(".github").join("workflows"), &root).expect(".github is not .git");
}

// --- root containment ----------------------------------------------------

#[test]
fn escaping_the_root_is_refused_lexically_and_by_traversal() {
    let dir = tmp();
    let root = root_of(&dir);

    let sibling = root.parent().unwrap().join("elsewhere").join("evil.txt");
    assert!(matches!(
        validate_path(&sibling, &root),
        Err(CoreError::PathEscapesRoot { .. })
    ));

    let traversal = root.join("..").join("evil.txt");
    assert!(
        matches!(
            validate_path(&traversal, &root),
            Err(CoreError::PathEscapesRoot { .. })
        ),
        "`..` components must be refused rather than normalised away"
    );
}

#[test]
fn relative_paths_are_refused_because_a_plan_carries_resolved_paths() {
    let dir = tmp();
    let root = root_of(&dir);
    assert!(validate_path(std::path::Path::new("relative.txt"), &root).is_err());
    assert!(validate_path(&root.join("ok.txt"), std::path::Path::new("rel")).is_err());
}

// --- apply ---------------------------------------------------------------

#[test]
fn apply_writes_what_the_plan_said_and_nothing_else() {
    let dir = tmp();
    let root = root_of(&dir);
    let target = root.join("sub").join("file.txt");

    let plan = WritePlan::new(
        PlanIntent::New,
        root.clone(),
        vec![
            FileOp::CreateDir {
                path: root.join("sub"),
            },
            FileOp::CreateFile {
                path: target.clone(),
                contents: "hello\n".to_owned(),
            },
        ],
    );

    let rep = CollectingReporter::new();
    let report = plan::apply(&plan, &rep).expect("apply");

    assert_eq!(report.outcome, ApplyOutcome::Completed);
    assert_eq!(report.applied.len(), 2);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello\n");

    let events = rep.events();
    assert!(matches!(
        events.first(),
        Some(Event::ApplyStarted { ops: 2 })
    ));
    assert!(matches!(
        events.last(),
        Some(Event::ApplyFinished { applied: 2, .. })
    ));
}

#[test]
fn a_plan_that_cannot_complete_writes_nothing_at_all() {
    let dir = tmp();
    let root = root_of(&dir);
    let good = root.join("first.txt");

    // The second op is invalid. Because every op is validated before any op
    // runs, the first must not have been written either.
    let plan = WritePlan::new(
        PlanIntent::New,
        root.clone(),
        vec![
            FileOp::CreateFile {
                path: good.clone(),
                contents: "a".to_owned(),
            },
            FileOp::CreateFile {
                path: root.join(".git").join("hooks").join("post-commit"),
                contents: "#!/bin/sh\n".to_owned(),
            },
        ],
    );

    let err = plan::apply(&plan, &NullReporter).expect_err("must refuse the whole plan");
    assert!(matches!(err, CoreError::PathInsideGitDir { .. }));
    assert!(
        !good.exists(),
        "the valid op must not have run — validation happens before any write"
    );
}

#[test]
fn creating_a_directory_that_exists_is_skipped_not_failed() {
    let dir = tmp();
    let root = root_of(&dir);

    let plan = WritePlan::new(
        PlanIntent::New,
        root.clone(),
        vec![FileOp::CreateDir { path: root.clone() }],
    );
    let report = plan::apply(&plan, &NullReporter).expect("idempotent");
    assert_eq!(report.applied.len(), 0);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].reason, SkipReason::DirectoryAlreadyExists);
}

#[test]
fn creating_a_file_that_exists_is_refused() {
    let dir = tmp();
    let root = root_of(&dir);
    let target = root.join("taken.txt");
    std::fs::write(&target, "mine").unwrap();

    let plan = WritePlan::new(
        PlanIntent::New,
        root.clone(),
        vec![FileOp::CreateFile {
            path: target.clone(),
            contents: "theirs".to_owned(),
        }],
    );
    let err = plan::apply(&plan, &NullReporter).expect_err("must not clobber");
    assert!(matches!(err, CoreError::TargetExists { .. }));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "mine");
}

#[test]
fn an_update_whose_file_changed_after_planning_is_refused() {
    let dir = tmp();
    let root = root_of(&dir);
    let target = root.join("f.toml");
    std::fs::write(&target, "original").unwrap();

    let plan = WritePlan::new(
        PlanIntent::Sync,
        root.clone(),
        vec![FileOp::UpdateFile {
            path: target.clone(),
            before: "original".to_owned(),
            after: "edited".to_owned(),
            reason: vibe_core::manifest::EditReason::FieldsUpdated,
        }],
    );

    // Someone else edits the file between plan and apply.
    std::fs::write(&target, "someone else got here first").unwrap();

    let err = plan::apply(&plan, &NullReporter).expect_err("stale plans must not apply");
    assert!(matches!(err, CoreError::PlanStale { .. }));
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "someone else got here first",
        "their edit must survive"
    );
}

// --- cancellation is a success outcome (ADR-0005 §2) --------------------

#[derive(Debug, Default)]
struct CancelImmediately;

impl Reporter for CancelImmediately {
    fn event(&self, _ev: Event) {}
    fn should_cancel(&self) -> bool {
        true
    }
}

#[test]
fn cancelling_returns_ok_and_says_where_it_stopped() {
    let dir = tmp();
    let root = root_of(&dir);

    let plan = WritePlan::new(
        PlanIntent::New,
        root.clone(),
        vec![
            FileOp::CreateFile {
                path: root.join("a.txt"),
                contents: "a".to_owned(),
            },
            FileOp::CreateFile {
                path: root.join("b.txt"),
                contents: "b".to_owned(),
            },
        ],
    );

    let report = plan::apply(&plan, &CancelImmediately).expect("cancellation is not an error");
    assert_eq!(report.outcome, ApplyOutcome::Cancelled { after_ops: 0 });
    assert!(report.applied.is_empty());
}

// --- vibe new, end to end ------------------------------------------------

#[test]
fn plan_new_produces_a_directory_a_vibe_dir_and_a_manifest() {
    let dir = tmp();
    let root = root_of(&dir);

    let registry = Registry::open(Config::default().with_clock(std::sync::Arc::new(
        vibe_core::config::FixedClock("2026-03-12".to_owned()),
    )));
    let req = NewRequest::new("macroring")
        .with_parent_dir(&root)
        .with_description(Some("nutrition tracking".to_owned()));

    let plan = registry.plan_new(&req).expect("plan");
    assert_eq!(plan.intent, PlanIntent::New);
    assert_eq!(plan.ops.len(), 3);

    let report = registry.apply(&plan, &NullReporter).expect("apply");
    assert_eq!(report.outcome, ApplyOutcome::Completed);

    let manifest = root.join("macroring").join(".vibe").join("project.toml");
    assert!(manifest.is_file(), "manifest should exist at {manifest:?}");

    let parsed = vibe_core::ManifestDocument::open(&manifest)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(parsed.project.name, "macroring");
    assert_eq!(
        parsed.project.description.as_deref(),
        Some("nutrition tracking")
    );
    assert_eq!(parsed.project.created.as_deref(), Some("2026-03-12"));
}

#[test]
fn vibe_new_refuses_to_adopt_an_existing_directory() {
    let dir = tmp();
    let root = root_of(&dir);
    std::fs::create_dir(root.join("taken")).unwrap();

    let registry = Registry::open(Config::default());
    let err = registry
        .plan_new(&NewRequest::new("taken").with_parent_dir(&root))
        .expect_err("`vibe new` is for projects that do not exist");
    assert!(matches!(err, CoreError::TargetExists { .. }));
}

/// A plan is data going *out*. If it could come back in, `apply` would accept
/// arbitrary file writes as input from whatever produced the JSON — which in a
/// desktop build is a webview (ADR-0005 §1).
#[test]
fn a_write_plan_serialises_but_cannot_be_deserialised() {
    let dir = tmp();
    let root = root_of(&dir);
    let plan = WritePlan::new(
        PlanIntent::New,
        root.clone(),
        vec![FileOp::CreateFile {
            path: root.join("x"),
            contents: "y".to_owned(),
        }],
    );

    let json = serde_json::to_string(&plan).expect("plans serialise for --json");
    assert!(json.contains("create_file"));

    // The other half of this invariant is the *absence* of a `Deserialize`
    // impl, which cannot be asserted from inside a passing test — the check is
    // that the following line does not compile:
    //
    //     fn needs_de<T: serde::de::DeserializeOwned>() {}
    //     needs_de::<WritePlan>();
    //
    // Verified by uncommenting it and observing E0277. Anyone who adds the
    // derive to make something convenient will find this comment first.
}
