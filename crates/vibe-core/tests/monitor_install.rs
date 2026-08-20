//! Controls for `vibe monitor install` — ADR-0011 §7b and §9.
//!
//! **Every control here plants its own home directory**, because
//! `install::plan` and `install::state` take one as a parameter. That is the
//! `list_prompts(.., user_home, ..)` precedent and it exists for this reason: a
//! resolver buried in core would make every assertion below depend on the real
//! user's `~/.claude/settings.json`, which is both unrepeatable and somebody's
//! actual configuration.
//!
//! # What is asserted on the BYTES rather than on a `Result`
//!
//! The refusal controls. *"`plan` returned an error"* and *"nothing was
//! written"* are different claims and only the second is the one that reaches
//! somebody's config (§7b). A build that refused after truncating the file
//! satisfies the first perfectly.

use std::path::{Path, PathBuf};

use vibe_core::monitor::{
    INSTALLED_EVENTS, InstallOutcome, InstallRequest, InstallState, SettingsTarget, WriterIdentity,
    install,
};
use vibe_core::{FileOp, NullReporter, plan};

/// A home with no `.claude` at all — the first-install case.
fn bare_home() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// A home whose `settings.json` holds `text`.
fn home_with(text: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let claude = dir.path().join(".claude");
    std::fs::create_dir_all(&claude).expect("mkdir");
    std::fs::write(claude.join("settings.json"), text).expect("seed");
    dir
}

fn settings_path(home: &Path) -> PathBuf {
    home.join(".claude").join("settings.json")
}

fn request(home: &Path) -> InstallRequest {
    // The command and sink are planted rather than resolved, for the same
    // reason `home` is: `current_exe()` and the data directory are facts about
    // the machine running the test.
    InstallRequest {
        home: home.to_path_buf(),
        command: home.join("bin").join("vibe"),
        sink: home.join("data").join("monitor"),
        identity: WriterIdentity::parse("vibe").expect("a valid identity"),
    }
}

/// A settings file a real user would have: four-space indent, their own hook,
/// and keys vibe knows nothing about.
const REALISTIC: &str = r#"{
    "permissions": {
        "allow": ["Bash(ls:*)"]
    },
    "hooks": {
        "SessionStart": [
            {
                "matcher": "*",
                "hooks": [
                    {
                        "type": "command",
                        "command": "echo",
                        "args": ["theirs"]
                    }
                ]
            }
        ]
    }
}
"#;

// ---------------------------------------------------------------------------
// The route
// ---------------------------------------------------------------------------

/// **The op names no path, and an op that does is beside it.**
///
/// ADR-0011 §7b: *a plan cannot express a write to an arbitrary place outside a
/// root because there is no field to put one in*. Asserting only that the
/// settings op returns `None` would pass against a build where `path()` returns
/// `None` for everything, so the paired half is an ordinary op from another
/// plan whose path is still there.
#[test]
fn the_settings_op_names_no_path_and_an_ordinary_op_still_does() {
    let home = bare_home();
    let planned = install::plan(&request(home.path())).expect("plans");

    let op = planned.plan.ops.first().expect("one op");
    assert!(
        matches!(op, FileOp::UpdateSettings { .. }),
        "install must route through UpdateSettings, got {op:?}"
    );
    assert!(
        op.path().is_none(),
        "the settings op named a path: {:?}. The containment argument in §7b \
         rests on there being no field to put one in.",
        op.path()
    );

    // The paired half. Without it, a build whose `path()` returned `None`
    // unconditionally would satisfy the assertion above.
    let ordinary = FileOp::CreateDir {
        path: home.path().join("somewhere"),
    };
    assert!(
        ordinary.path().is_some(),
        "an ordinary op stopped naming its path, so the assertion above is \
         about `path()` being useless rather than about this variant"
    );
}

/// The destination `apply` will use is the one the dry run showed.
#[test]
fn the_op_resolves_to_the_user_settings_file_under_the_plan_root() {
    let home = bare_home();
    let planned = install::plan(&request(home.path())).expect("plans");
    let op = planned.plan.ops.first().expect("one op");

    let resolved = op.resolved_path(&planned.plan.root);
    assert_eq!(resolved, settings_path(home.path()));
    assert_eq!(resolved, planned.target_path);
    assert_eq!(
        planned.plan.root,
        SettingsTarget::User.containment_root(home.path()),
        "the plan's containment root must be the .claude directory, or \
         `validate_path` is bounding the write against something else"
    );
}

// ---------------------------------------------------------------------------
// Install, re-install, upgrade
// ---------------------------------------------------------------------------

/// **First install, on a home with no `.claude` at all.**
///
/// The before side is `None` — which is a real state and not an error — and
/// every installed event ends up carrying vibe's group.
#[test]
fn a_first_install_writes_all_five_events_where_nothing_existed() {
    let home = bare_home();
    let planned = install::plan(&request(home.path())).expect("plans");

    assert!(matches!(planned.outcome, InstallOutcome::Installed { .. }));
    match planned.plan.ops.first().expect("one op") {
        FileOp::UpdateSettings { before, .. } => assert!(
            before.is_none(),
            "a file that does not exist must plan with no before side, or the \
             dry run implies there was something to replace"
        ),
        other => panic!("wrong op: {other:?}"),
    }

    plan::apply(&planned.plan, &NullReporter).expect("applies");

    let written = std::fs::read_to_string(settings_path(home.path())).expect("written");
    let doc: serde_json::Value = serde_json::from_str(&written).expect("valid json");
    for event in INSTALLED_EVENTS {
        let groups = doc["hooks"][event].as_array().unwrap_or_else(|| {
            panic!("no groups for {event}: {written}");
        });
        assert_eq!(groups.len(), 1, "{event} should carry exactly vibe's group");
    }
}

/// **Re-install is `Unchanged` with an EMPTY plan.**
///
/// §7b makes re-install the normal path. A plan that rewrote the file with
/// identical bytes would still race `PlanStale`, still move the mtime, and
/// still render an empty diff.
#[test]
fn re_installing_is_unchanged_and_plans_nothing() {
    let home = bare_home();
    let first = install::plan(&request(home.path())).expect("plans");
    plan::apply(&first.plan, &NullReporter).expect("applies");
    let after_first = std::fs::read(settings_path(home.path())).expect("read");

    let second = install::plan(&request(home.path())).expect("plans");
    assert_eq!(second.outcome, InstallOutcome::Unchanged);
    assert!(
        second.plan.is_empty(),
        "re-install planned {} ops against an identical config",
        second.plan.ops.len()
    );

    // And the paired half: the bytes really are untouched, not merely
    // "planned as unchanged".
    assert_eq!(
        std::fs::read(settings_path(home.path())).expect("read"),
        after_first
    );
}

/// **A changed sink upgrades in place rather than adding a second group.**
///
/// The hook is found by the identity it declares, never by its command path
/// (§7a). A build keying on the command would fail to find the old group here
/// and append a second one — two writers, one identity.
#[test]
fn a_moved_sink_upgrades_the_existing_group_rather_than_adding_one() {
    let home = bare_home();
    let first = install::plan(&request(home.path())).expect("plans");
    plan::apply(&first.plan, &NullReporter).expect("applies");

    let mut moved = request(home.path());
    moved.sink = home.path().join("data").join("elsewhere");
    moved.command = home.path().join("bin2").join("vibe");
    let second = install::plan(&moved).expect("plans");

    assert!(matches!(second.outcome, InstallOutcome::Upgraded { .. }));
    plan::apply(&second.plan, &NullReporter).expect("applies");

    let written = std::fs::read_to_string(settings_path(home.path())).expect("written");
    let doc: serde_json::Value = serde_json::from_str(&written).expect("valid json");
    for event in INSTALLED_EVENTS {
        assert_eq!(
            doc["hooks"][event].as_array().expect("groups").len(),
            1,
            "{event} grew a second group instead of being upgraded: {written}"
        );
    }
    assert!(
        written.contains("elsewhere"),
        "the new sink never reached the file: {written}"
    );
}

/// The user's own hook and unknown keys survive, and so does their indentation.
#[test]
fn a_users_own_configuration_survives_the_edit() {
    let home = home_with(REALISTIC);
    let planned = install::plan(&request(home.path())).expect("plans");
    plan::apply(&planned.plan, &NullReporter).expect("applies");

    let written = std::fs::read_to_string(settings_path(home.path())).expect("written");
    let doc: serde_json::Value = serde_json::from_str(&written).expect("valid json");

    assert_eq!(
        doc["permissions"]["allow"][0], "Bash(ls:*)",
        "an unknown key was dropped: {written}"
    );
    assert!(
        written.contains("theirs"),
        "the user's own hook was deleted: {written}"
    );
    // Their group and vibe's, side by side, never merged into one — a
    // non-matching `matcher` on a shared group suppresses every hook in it.
    assert_eq!(
        doc["hooks"]["SessionStart"]
            .as_array()
            .expect("groups")
            .len(),
        2,
        "vibe joined the user's group instead of adding its own: {written}"
    );
    assert!(
        written.contains("\n        \""),
        "the file's four-space indentation was not preserved: {written}"
    );
}

// ---------------------------------------------------------------------------
// Refusals, asserted on the bytes
// ---------------------------------------------------------------------------

/// **An unparseable config is refused and the bytes are IDENTICAL afterwards.**
///
/// The syntactic case is the likely one: users edit this file by hand and §7
/// measured the loader refusing comments and trailing commas. Asserted on disk
/// rather than on the `Result`, because those are different claims.
#[test]
fn an_unparseable_config_is_refused_and_nothing_is_written() {
    let hand_edited = "{\n  // a comment Claude Code also refuses\n  \"hooks\": {}\n}\n";
    let home = home_with(hand_edited);
    let before = std::fs::read(settings_path(home.path())).expect("read");

    let err = install::plan(&request(home.path())).expect_err("must refuse");
    assert_eq!(err.code(), "VIBE_E_SETTINGS_REFUSED", "{err}");

    assert_eq!(
        std::fs::read(settings_path(home.path())).expect("read"),
        before,
        "the bytes changed on a refusal"
    );

    // Paired: the same home with the comment removed is accepted, so the
    // refusal above is about the syntax rather than about the fixture being
    // unreachable for some other reason.
    let ok = home_with("{\n  \"hooks\": {}\n}\n");
    install::plan(&request(ok.path())).expect("a strict-JSON file must be accepted");
}

/// **A stale plan is refused at apply, in both directions.**
///
/// The `before: None` direction is the half a one-sided version drops: a
/// settings file that appeared between planning and applying is somebody's
/// configuration, and writing over it would discard a file the user never saw
/// in the diff.
#[test]
fn a_file_that_appeared_after_planning_is_not_overwritten() {
    let home = bare_home();
    let planned = install::plan(&request(home.path())).expect("plans");

    // Someone configures Claude Code between the dry run and the apply.
    let path = settings_path(home.path());
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, REALISTIC).expect("seed");

    let err = plan::apply(&planned.plan, &NullReporter).expect_err("must refuse");
    assert_eq!(err.code(), "VIBE_E_PLAN_STALE", "{err}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        REALISTIC,
        "the file that appeared was overwritten"
    );
}

/// The other direction: the file changed after planning.
#[test]
fn a_file_that_changed_after_planning_is_not_overwritten() {
    let home = home_with(REALISTIC);
    let planned = install::plan(&request(home.path())).expect("plans");

    let path = settings_path(home.path());
    let edited = REALISTIC.replace("Bash(ls:*)", "Bash(cat:*)");
    std::fs::write(&path, &edited).expect("edit");

    let err = plan::apply(&planned.plan, &NullReporter).expect_err("must refuse");
    assert_eq!(err.code(), "VIBE_E_PLAN_STALE", "{err}");
    assert_eq!(std::fs::read_to_string(&path).expect("read"), edited);
}

/// A traversal identity is refused before anything is planned.
#[test]
fn a_traversal_identity_never_reaches_a_plan() {
    for hostile in ["../escape", "a/b", "a:b", "a__b"] {
        assert!(
            WriterIdentity::parse(hostile).is_err(),
            "{hostile:?} was accepted as an identity"
        );
    }
    // Paired, or the loop above passes against a parser that rejects
    // everything.
    WriterIdentity::parse("vibe-monitor").expect("a valid identity must be accepted");
}

// ---------------------------------------------------------------------------
// The emitted config
// ---------------------------------------------------------------------------

/// **`async` is never written true, asserted on the bytes that ship.**
///
/// §2 round 3b measured an `async: true` hook killed with its start written and
/// its end never — silent non-delivery through the mechanism installed to
/// prevent it, one boolean away. Asserted on the file rather than on a constant
/// in the source: what ships is what install writes.
#[test]
fn the_emitted_hook_never_declares_async() {
    let home = bare_home();
    let planned = install::plan(&request(home.path())).expect("plans");
    plan::apply(&planned.plan, &NullReporter).expect("applies");

    let written = std::fs::read_to_string(settings_path(home.path())).expect("written");
    let doc: serde_json::Value = serde_json::from_str(&written).expect("valid json");

    for event in INSTALLED_EVENTS {
        let hook = &doc["hooks"][event][0]["hooks"][0];
        assert_eq!(hook["async"], serde_json::Value::Bool(false), "{event}");
        assert_eq!(
            hook["asyncRewake"],
            serde_json::Value::Bool(false),
            "{event}"
        );
        // The exec form is what keeps a shell out of the channel, since no
        // `shell` value means "no shell" exists.
        assert!(
            hook["args"].is_array(),
            "the hook is not in the args exec form: {hook}"
        );
        assert!(
            hook.get("shell").is_none(),
            "a `shell` key was written: {hook}"
        );
    }
}

// ---------------------------------------------------------------------------
// The three states — paired, per §9
// ---------------------------------------------------------------------------

/// **Three states, and each is reachable.**
///
/// §9's rule against one-sided controls, applied to the state this whole read
/// exists for. A build that returned one state unconditionally satisfies any
/// single assertion here perfectly, so all three are asserted in one test
/// against homes that differ only in what they contain.
#[test]
fn not_installed_degraded_and_healthy_are_three_distinct_states() {
    // 1. Nothing wired.
    let bare = bare_home();
    let identity = WriterIdentity::parse("vibe").expect("valid");
    assert_eq!(
        install::state(bare.path(), &identity).expect("reads"),
        InstallState::NotInstalled,
        "a home with no settings file must read as not installed"
    );

    // A settings file that exists but declares nothing of ours is ALSO not
    // installed — and this is the half that separates "no file" from "no hook".
    let theirs = home_with(REALISTIC);
    assert_eq!(
        install::state(theirs.path(), &identity).expect("reads"),
        InstallState::NotInstalled,
        "someone else's config must not read as our install"
    );

    // 2. Installed, but what it names is gone. The request points `command`
    //    and `sink` at paths that were never created.
    let degraded = bare_home();
    let planned = install::plan(&request(degraded.path())).expect("plans");
    plan::apply(&planned.plan, &NullReporter).expect("applies");
    match install::state(degraded.path(), &identity).expect("reads") {
        InstallState::Degraded {
            command_present,
            sink_present,
            ..
        } => {
            assert!(!command_present, "the command should be missing");
            assert!(!sink_present, "the sink should be missing");
        }
        other => panic!("expected Degraded, got {other:?}"),
    }

    // 3. Installed and everything it names exists.
    let healthy = bare_home();
    let req = request(healthy.path());
    std::fs::create_dir_all(req.command.parent().expect("parent")).expect("mkdir");
    std::fs::write(&req.command, b"not really a binary").expect("plant");
    std::fs::create_dir_all(&req.sink).expect("mkdir");
    let planned = install::plan(&req).expect("plans");
    plan::apply(&planned.plan, &NullReporter).expect("applies");
    match install::state(healthy.path(), &identity).expect("reads") {
        InstallState::Healthy { .. } => {}
        other => panic!("expected Healthy, got {other:?}"),
    }
}

/// **A moved binary is `Degraded`, not `Healthy`, and it is the sink's
/// neighbour that stays present.**
///
/// The two booleans fail for different reasons and need different repairs. One
/// flag would make a user guess which, and a build that ANDed them into a single
/// "broken" would pass a control that only ever removed both.
#[test]
fn a_moved_binary_degrades_while_the_sink_stays_present() {
    let home = bare_home();
    let req = request(home.path());
    std::fs::create_dir_all(req.command.parent().expect("parent")).expect("mkdir");
    std::fs::write(&req.command, b"binary").expect("plant");
    std::fs::create_dir_all(&req.sink).expect("mkdir");

    let planned = install::plan(&req).expect("plans");
    plan::apply(&planned.plan, &NullReporter).expect("applies");

    // The upgrade that moves the binary out from under a config naming it.
    std::fs::remove_file(&req.command).expect("remove");

    let identity = WriterIdentity::parse("vibe").expect("valid");
    match install::state(home.path(), &identity).expect("reads") {
        InstallState::Degraded {
            command_present,
            sink_present,
            ..
        } => {
            assert!(!command_present, "the moved binary should read as missing");
            assert!(
                sink_present,
                "the sink is still there and must read as present, or the two \
                 fields have been collapsed into one verdict"
            );
        }
        other => panic!("expected Degraded, got {other:?}"),
    }
}

/// A malformed config is **not** `NotInstalled`.
///
/// Reporting *nothing is wired* about a file nothing was learned from is
/// ADR-0001 §3b's defect at the config layer: a failed read producing a
/// confident answer.
#[test]
fn an_unreadable_config_is_an_error_rather_than_not_installed() {
    let home = home_with("{ not json at all");
    let identity = WriterIdentity::parse("vibe").expect("valid");
    let err = install::state(home.path(), &identity).expect_err("must not answer");
    assert_eq!(err.code(), "VIBE_E_SETTINGS_REFUSED", "{err}");
}
