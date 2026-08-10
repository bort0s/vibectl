//! The forward-compatibility contract for `[agents]`, asserted rather than
//! assumed.
//!
//! ADR-0006 §2 justifies putting `installed` in `.vibe/project.toml` rather than
//! in a third file with one sentence: *"Existing manifests parse unchanged; an
//! older binary reads a manifest containing it, ignores the table, preserves it
//! on write, and warns once. That is the forward-compatibility path working as
//! designed, and it is why the table goes in the manifest rather than a third
//! file."*
//!
//! That sentence is the entire justification for the design. If any half of it
//! is false, the decision was wrong — so each half gets a test.
//!
//! # Why this file exists when `round_trip.rs` already tests unknown keys
//!
//! `round_trip.rs` proves the *generic* property: a key this build never heard
//! of survives an edit to some other key. That is necessary and not sufficient
//! here, because `[agents]` is the first table this build both **knows about**
//! and **writes to**. The generic property covers the table nobody addresses;
//! it says nothing about the table we address every time `vibe agents add`
//! runs.
//!
//! The dangerous write path is the one that knows just enough:
//!
//! ```text
//! [agents]
//! installed = ["a", "b"]     # this build writes this
//! pinned_rev = "9c1d4e2"     # a 1.2 build writes this; we have never heard of it
//! ```
//!
//! A `vibe agents add` implemented by building an `Agents` struct and
//! serialising it back over the table produces exactly the right `installed`
//! array and silently deletes `pinned_rev`. The manifest still parses, still
//! validates, still round-trips — and the 1.2 build that wrote it has lost data
//! it had every right to expect back. `negative_control_*` below is that
//! implementation, kept in the test suite so the property has something to be
//! measured against.

use vibe_core::manifest::{EditReason, FieldEdit};
use vibe_core::{FileOp, ManifestDocument, SchemaVersion};

const PATH: &str = "/tmp/proj/.vibe/project.toml";

/// A key inside `[agents]` that this build has never heard of, as a 1.2 build
/// might write it. The sentinel every assertion below hunts for.
const FUTURE_KEY: &str = "pinned_rev";
const FUTURE_VALUE: &str = "9c1d4e2f8a7b3c5d6e0f1a2b3c4d5e6f7a8b9c0d";
const FUTURE_COMMENT: &str = "# pinned by a build that does not exist yet";

/// A manifest from a build one minor ahead of this one.
fn manifest_from_the_future() -> String {
    format!(
        r#"# Managed by vibe.
schema_version = "1.2"

[project]
name = "macroring"
status = "active"

[stack]
runtime = "rust@1.85"

# agents this project declares
[agents]
installed = ["engineering-code-reviewer", "engineering-rust-refactoring-specialist"]
{FUTURE_COMMENT}
{FUTURE_KEY} = "{FUTURE_VALUE}"
"#
    )
}

/// A manifest at the version this build writes.
fn manifest_at_current() -> String {
    r#"schema_version = "1.1"

[project]
name = "macroring"
status = "active"

[agents]
installed = ["engineering-code-reviewer"]
"#
    .to_owned()
}

fn rendered_after(text: &str, edit: FieldEdit) -> String {
    let mut doc = ManifestDocument::from_text(PATH, text).expect("valid TOML");
    doc.apply(edit).expect("edit applies");
    doc.render()
}

// --- half one: the table parses, and what it says is what we read -------

#[test]
fn a_manifest_with_agents_parses_and_reports_what_it_declares() {
    let doc = ManifestDocument::from_text(PATH, &manifest_at_current()).unwrap();
    let manifest = doc.parse().expect("parses");

    assert_eq!(
        manifest.agents.installed,
        vec!["engineering-code-reviewer".to_owned()]
    );
    assert_eq!(manifest.schema_version, SchemaVersion::new(1, 1));

    // `installed` is a known key now, so it must NOT be reported as unknown.
    // If it were, `vibe show` would tell the user this build does not
    // understand a table it reads and writes on every `agents add`.
    assert!(
        !manifest
            .unknown
            .iter()
            .any(|u| u.dotted_path.starts_with("agents")),
        "known agents keys reported as unknown: {:?}",
        manifest.unknown
    );
}

#[test]
fn a_manifest_without_agents_parses_unchanged_and_declares_nothing() {
    // "Existing manifests parse unchanged." An absent table is an empty
    // declaration, never an error — every manifest written before 1.1 is this.
    let text = "schema_version = \"1.0\"\n\n[project]\nname = \"old\"\n";
    let manifest = ManifestDocument::from_text(PATH, text)
        .unwrap()
        .parse()
        .expect("a 1.0 manifest must still parse");

    assert!(manifest.agents.installed.is_empty());
    assert_eq!(manifest.schema_version, SchemaVersion::new(1, 0));
}

#[test]
fn adding_agents_is_a_minor_bump_not_a_major_one() {
    // The claim ADR-0006 §2 makes about ADR-0002 §2, asserted directly. If
    // `[agents]` had needed a major bump, every deployed binary would refuse
    // every manifest that mentions an agent, and the table would have had to
    // live in a third file.
    assert_eq!(SchemaVersion::CURRENT, SchemaVersion::new(1, 1));
    assert!(SchemaVersion::new(1, 0).compat().is_writable());
    assert!(SchemaVersion::new(1, 1).compat().is_writable());
    // And a build one minor further on is still readable, which is what makes
    // the rest of this file a real scenario rather than a hypothetical.
    assert!(SchemaVersion::new(1, 2).compat().is_writable());
}

// --- half two: writing `installed` preserves what we do not know --------

#[test]
fn writing_installed_preserves_an_unknown_key_in_the_same_table() {
    // The sharp case. This build addresses `agents.installed`; a 1.2 build put
    // `agents.pinned_rev` beside it. Editing one must not disturb the other.
    let after = rendered_after(
        &manifest_from_the_future(),
        FieldEdit::ReplaceAgentsInstalled(vec!["security-appsec-engineer".to_owned()]),
    );

    assert!(
        after.contains(FUTURE_KEY),
        "editing `installed` dropped `{FUTURE_KEY}`\n--- after ---\n{after}"
    );
    assert!(
        after.contains(FUTURE_VALUE),
        "editing `installed` dropped the value of `{FUTURE_KEY}`\n--- after ---\n{after}"
    );
    assert!(
        after.contains(FUTURE_COMMENT),
        "editing `installed` dropped the comment on `{FUTURE_KEY}`\n--- after ---\n{after}"
    );
    // And it did the job it was asked to do.
    assert!(after.contains("security-appsec-engineer"));
    assert!(!after.contains("engineering-code-reviewer"));
}

#[test]
fn writing_installed_preserves_the_rest_of_the_manifest() {
    let after = rendered_after(
        &manifest_from_the_future(),
        FieldEdit::ReplaceAgentsInstalled(vec!["a".to_owned()]),
    );
    for kept in [
        "# Managed by vibe.",
        "# agents this project declares",
        "macroring",
        "rust@1.85",
    ] {
        assert!(after.contains(kept), "dropped {kept:?}\n{after}");
    }
}

#[test]
fn creating_the_agents_table_in_an_older_manifest_leaves_everything_else_alone() {
    // `vibe agents add` in a project whose manifest predates 1.1. The table
    // does not exist yet, so this is an insert rather than an edit — a
    // different code path, and the one most likely to reformat the file.
    let text = "# hand-written, do not mangle\nschema_version = \"1.0\"\n\n\
                [project]\nname = \"old\"  # the slug\nstatus = \"active\"\n";
    let after = rendered_after(
        text,
        FieldEdit::ReplaceAgentsInstalled(vec!["engineering-code-reviewer".to_owned()]),
    );

    assert!(after.contains("# hand-written, do not mangle"));
    assert!(after.contains("# the slug"));
    assert!(after.contains("engineering-code-reviewer"));
    assert!(
        ManifestDocument::from_text(PATH, &after)
            .unwrap()
            .parse()
            .unwrap()
            .agents
            .installed
            .len()
            == 1
    );
}

/// A manifest that declares 1.0 and then gains a 1.1 feature is lying about
/// itself, and the lie is load-bearing: `compat()` is what a build consults to
/// decide whether to warn. Writing `[agents]` therefore migrates the version.
#[test]
fn writing_agents_into_a_ten_manifest_migrates_the_declared_version() {
    let text = "schema_version = \"1.0\"\n\n[project]\nname = \"old\"\n";
    let mut doc = ManifestDocument::from_text(PATH, text).unwrap();
    doc.apply(FieldEdit::ReplaceAgentsInstalled(vec!["a".to_owned()]))
        .unwrap();
    doc.apply(FieldEdit::SetSchemaVersion(SchemaVersion::CURRENT))
        .unwrap();

    let op = doc
        .into_op(EditReason::SchemaMigration {
            from: SchemaVersion::new(1, 0),
            to: SchemaVersion::CURRENT,
        })
        .expect("the document changed");

    let FileOp::UpdateFile { after, reason, .. } = &op else {
        panic!("expected an update, got {op:?}");
    };
    assert!(after.contains(r#"schema_version = "1.1""#), "{after}");
    // The migration is a *visible* line in `--dry-run`, not an invisible side
    // effect of an unrelated command (ADR-0002, and `EditReason` exists for it).
    assert!(matches!(reason, EditReason::SchemaMigration { .. }));
}

/// The version bump must not be a silent downgrade of a *newer* manifest.
#[test]
fn writing_agents_never_lowers_a_newer_declared_version() {
    let text = manifest_from_the_future();
    let after = rendered_after(
        &text,
        FieldEdit::ReplaceAgentsInstalled(vec!["a".to_owned()]),
    );
    assert!(
        after.contains(r#"schema_version = "1.2""#),
        "editing a 1.2 manifest must leave its version alone; a build that \
         rewrote it to 1.1 would be claiming the file lost features it still \
         has\n{after}"
    );
}

// --- the negative control ----------------------------------------------
//
// Everything above asserts that *our* write path preserves. On its own that
// proves nothing about whether preserving is hard: if the obvious
// implementation also passed, the whole `toml_edit`-addressed-mutation design
// would be ceremony.
//
// So here is the obvious implementation — the one someone writes when they know
// about `[agents]` and reach for the type they already have. It is the write
// path ADR-0006 §2's promise fails under, and it must lose.

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct NaiveAgents {
    installed: Vec<String>,
}

/// Set `installed` by rebuilding the `[agents]` table from a typed struct.
///
/// This is not a straw man. It reads the table, produces a correct `installed`
/// array, and writes valid TOML — which is exactly why it is dangerous. The
/// data it destroys is the data it was never taught to carry.
fn naive_set_installed(text: &str, installed: Vec<String>) -> String {
    let mut doc: toml_edit::DocumentMut = text.parse().unwrap();
    let existing: NaiveAgents = doc
        .get("agents")
        .and_then(|item| item.as_table())
        .map(|t| toml_edit::de::from_str(&t.to_string()).unwrap_or_default())
        .unwrap_or_default();
    let _ = existing.installed;

    let replacement = NaiveAgents { installed };
    let rendered = toml_edit::ser::to_string(&replacement).unwrap();
    let table: toml_edit::DocumentMut = rendered.parse().unwrap();
    doc["agents"] = toml_edit::Item::Table(table.as_table().clone());
    doc.to_string()
}

#[test]
fn negative_control_a_write_path_that_knows_agents_drops_what_it_does_not_know() {
    let text = manifest_from_the_future();
    let naive = naive_set_installed(&text, vec!["security-appsec-engineer".to_owned()]);

    // It gets the visible job right, which is the whole problem.
    assert!(naive.contains("security-appsec-engineer"));
    assert!(
        naive.parse::<toml_edit::DocumentMut>().is_ok(),
        "still valid TOML — nothing downstream would notice"
    );

    // And silently deletes the rest of the table.
    assert!(
        !naive.contains(FUTURE_KEY),
        "the naive path was expected to drop `{FUTURE_KEY}` but kept it. If \
         this now passes, re-examine whether the addressed-mutation design is \
         still buying anything:\n{naive}"
    );
    assert!(!naive.contains(FUTURE_COMMENT));

    // Ours, on the identical input, keeps both.
    let ours = rendered_after(
        &text,
        FieldEdit::ReplaceAgentsInstalled(vec!["security-appsec-engineer".to_owned()]),
    );
    assert!(ours.contains(FUTURE_KEY), "ours dropped `{FUTURE_KEY}`");
    assert!(ours.contains(FUTURE_COMMENT));
    assert!(ours.contains("security-appsec-engineer"));
}

/// The same contrast for the table as a whole, which is the case an *older*
/// binary hits: it has never heard of `[agents]` at all.
#[test]
fn negative_control_an_older_binary_preserves_a_table_it_cannot_read() {
    let text = manifest_from_the_future();

    // Simulating the older binary exactly: an edit that addresses a key in a
    // different table, which is every command a pre-1.1 build has.
    let ours = rendered_after(&text, FieldEdit::SetStackRuntime("rust@1.90".to_owned()));
    assert!(ours.contains("[agents]"), "the table itself was dropped");
    assert!(ours.contains("engineering-code-reviewer"));
    assert!(ours.contains(FUTURE_KEY));
    assert!(ours.contains("rust@1.90"), "the edit did not apply");
}
