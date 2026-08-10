//! The single most valuable property in the project, at the level a user
//! actually experiences it.
//!
//! P1 built comment preservation and unknown-key survival and property-tested
//! them at the `ManifestDocument` level. `sync` is their **first production
//! caller**, and a property that holds in a unit test but not through the real
//! command is worth nothing. A user's hand-written comment silently eaten by
//! `sync` is the failure they would notice and never be able to explain.
//!
//! So this runs the binary, on a manifest hand-edited into shapes the
//! implementation was not written against, and checks the file afterwards.

use std::path::Path;
use std::process::Command;

fn vibe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vibe"))
}

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, contents).expect("write");
}

/// A manifest edited the way people actually edit files: comments in odd
/// places, a key from a schema minor this build does not know, alignment the
/// author cared about, and a table order that is not the one `vibe new` emits.
const HAND_EDITED: &str = r#"# ---------------------------------------------------------------
# macroring - nutrition tracker
# I keep my own notes in here. Do not eat them.
# ---------------------------------------------------------------
schema_version = "1.4"

[project]
name        = "macroring"          # aligned on purpose
status      = "active"
created     = "2026-03-12"
description = "Mobile-first PWA for nutrition tracking"

# a section from a build that does not exist yet
[ai]
model   = "something-future"
summary = "generated last tuesday"

[context]
decisions = [
  "iOS-native design system",
  "client-side economy enforcement (known debt)",   # revisit before launch
]
next = ["validate draggable sheet on physical device"]
owner = "riccardo"

[deploy]
env_required = ["SUPABASE_URL"]
regions      = ["fra1", "iad1"]   # future minor: not understood by this build

[stack]
runtime = ""              # sync fills this - keep my note
frameworks = []           # and this one
"#;

/// Everything that must survive, verbatim.
const MUST_SURVIVE: &[&str] = &[
    "# macroring - nutrition tracker",
    "# I keep my own notes in here. Do not eat them.",
    "# aligned on purpose",
    "# a section from a build that does not exist yet",
    "# revisit before launch",
    "# future minor: not understood by this build",
    "[ai]",
    "something-future",
    "generated last tuesday",
    "owner = \"riccardo\"",
    "regions",
    "fra1",
    "iad1",
    "iOS-native design system",
    "client-side economy enforcement (known debt)",
    "validate draggable sheet on physical device",
    // These two sit on keys `sync` DOES write, which is what makes this
    // non-vacuous. Without them every sentinel would be on an untouched key,
    // and an implementation that discarded the decor of the key it edits would
    // pass - verified by sabotage, which is how the omission was found.
    "# sync fills this - keep my note",
    "# and this one",
];

fn project_with_hand_edits() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("macroring");
    write(&root, ".vibe/project.toml", HAND_EDITED);
    // Real detectable content, so `sync` has something to write and this test
    // is not vacuously about a no-op.
    write(
        &root,
        "package.json",
        r#"{"name":"macroring","engines":{"node":">=22"},"dependencies":{"react":"^19.0.0","@supabase/supabase-js":"^2.0.0"}}"#,
    );
    write(&root, ".env.example", "SUPABASE_URL=\nSUPABASE_ANON_KEY=\n");
    dir
}

#[test]
fn sync_writes_detected_values_and_eats_nothing() {
    let dir = project_with_hand_edits();
    let root = dir.path().join("macroring");
    let manifest = root.join(".vibe/project.toml");

    let out = vibe().arg("sync").arg(&root).output().expect("run vibe");
    assert!(
        out.status.success(),
        "sync failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let after = std::fs::read_to_string(&manifest).expect("read back");

    // It actually did something, or this test proves nothing at all.
    assert!(
        after.contains(r#"runtime = "node@22""#),
        "sync should have filled the empty runtime:\n{after}"
    );
    assert!(after.contains("react@19"), "frameworks should be written");

    // And it disturbed nothing else.
    for needle in MUST_SURVIVE {
        assert!(
            after.contains(needle),
            "sync destroyed {needle:?}\n--- after ---\n{after}"
        );
    }

    // The unknown table survives as a table, not as flattened debris.
    assert!(
        after.contains("[ai]"),
        "the unknown [ai] table must survive"
    );
    assert!(
        after.matches("[context]").count() == 1,
        "no table may be duplicated:\n{after}"
    );
}

#[test]
fn sync_preserves_a_minor_newer_schema_version_and_says_so_once() {
    let dir = project_with_hand_edits();
    let root = dir.path().join("macroring");

    let out = vibe().arg("sync").arg(&root).output().expect("run vibe");
    assert!(out.status.success());

    let after = std::fs::read_to_string(root.join(".vibe/project.toml")).expect("read");
    // A minor-newer manifest is writable, and its version is not rewritten
    // downward. Silently "correcting" 1.4 to 1.0 would discard the author's
    // statement about their own file.
    assert!(
        after.contains(r#"schema_version = "1.4""#),
        "the declared schema version must not be rewritten:\n{after}"
    );

    // Coalesced to one line, not one per manifest.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr.matches("newer schema minor").count(),
        1,
        "the minor-newer warning should appear exactly once:\n{stderr}"
    );
}

#[test]
fn sync_is_idempotent() {
    let dir = project_with_hand_edits();
    let root = dir.path().join("macroring");
    let manifest = root.join(".vibe/project.toml");

    assert!(
        vibe()
            .arg("sync")
            .arg(&root)
            .status()
            .expect("run")
            .success()
    );
    let once = std::fs::read_to_string(&manifest).expect("read");

    assert!(
        vibe()
            .arg("sync")
            .arg(&root)
            .status()
            .expect("run")
            .success()
    );
    let twice = std::fs::read_to_string(&manifest).expect("read");

    // A second sync that rewrites the file would make `vibe sync` produce a
    // git diff on every run, which is the thing that stops people running it.
    assert_eq!(once, twice, "sync must be idempotent");
}

#[test]
fn sync_dry_run_writes_nothing_at_all() {
    let dir = project_with_hand_edits();
    let root = dir.path().join("macroring");
    let manifest = root.join(".vibe/project.toml");
    let before = std::fs::read_to_string(&manifest).expect("read");

    let out = vibe()
        .args(["sync", "--dry-run"])
        .arg(&root)
        .output()
        .expect("run vibe");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("update file"));

    assert_eq!(
        std::fs::read_to_string(&manifest).expect("read"),
        before,
        "--dry-run modified the manifest"
    );
}

/// The rule from the ADR-0003 §6 amendment, at the command level.
#[test]
fn sync_never_replaces_a_populated_field_with_an_absence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("proj");

    // A manifest that already records a remote and a deploy URL. Nothing on
    // disk can re-derive either: there is no git repository here, and no
    // deploy config.
    write(
        &root,
        ".vibe/project.toml",
        r#"schema_version = "1.0"

[project]
name = "proj"
status = "active"

[repo]
remote = "github.com/bort0s/proj"

[deploy]
url = "https://proj.example.com"

[stack]
runtime = "node@22"
"#,
    );
    write(&root, "package.json", r#"{"name":"proj"}"#);

    let out = vibe().arg("sync").arg(&root).output().expect("run vibe");
    assert!(out.status.success());

    let after = std::fs::read_to_string(root.join(".vibe/project.toml")).expect("read");
    // Detection has nothing to say about any of these. A field populated last
    // week must not go empty this week because this machine could not detect
    // it — that is data loss wearing a sync's clothes.
    assert!(
        after.contains(r#"remote = "github.com/bort0s/proj""#),
        "the recorded remote was destroyed:\n{after}"
    );
    assert!(
        after.contains(r#"url = "https://proj.example.com""#),
        "the recorded deploy url was destroyed:\n{after}"
    );
    assert!(
        after.contains(r#"runtime = "node@22""#),
        "a more precise recorded runtime must not be downgraded:\n{after}"
    );
}

#[test]
fn sync_refuses_a_manifest_from_a_newer_major_without_touching_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("ahead");
    let body = "schema_version = \"9.0\"\n\n[project]\nname = \"ahead\"\n# still mine\n";
    write(&root, ".vibe/project.toml", body);
    write(&root, "package.json", r#"{"name":"ahead"}"#);

    let out = vibe().arg("sync").arg(&root).output().expect("run vibe");
    assert_eq!(out.status.code(), Some(1), "a major mismatch is a failure");
    assert!(String::from_utf8_lossy(&out.stderr).contains("upgrade vibe"));

    assert_eq!(
        std::fs::read_to_string(root.join(".vibe/project.toml")).expect("read"),
        body,
        "a manifest this build cannot read must not be written to at all"
    );
}
