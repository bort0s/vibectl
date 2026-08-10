//! Fixture-based coverage of the schema rules from ADR-0002, and snapshots of
//! what actually lands on disk.

use vibe_core::manifest::{Compat, FieldEdit, SchemaVersion};
use vibe_core::{CoreError, ManifestDocument, Status};

const PATH: &str = "/tmp/proj/.vibe/project.toml";

fn doc(text: &str) -> ManifestDocument {
    ManifestDocument::from_text(PATH, text).expect("fixture is valid TOML")
}

#[test]
fn scaffolded_manifest_is_stable() {
    let d = ManifestDocument::scaffold(PATH, "macroring", "2026-03-12");
    insta::assert_snapshot!("scaffold", d.render());
}

#[test]
fn scaffolded_manifest_parses_back() {
    let d = ManifestDocument::scaffold(PATH, "macroring", "2026-03-12");
    let m = d.parse().expect("scaffold output must parse");
    assert_eq!(m.project.name, "macroring");
    assert_eq!(m.project.status, Status::Active);
    assert!(!m.project.archived);
    assert_eq!(m.schema_version, SchemaVersion::CURRENT);
    // Empty scaffolded fields read as absent rather than as the empty string:
    // "not determined" is not a value, and the tool must not present one.
    assert_eq!(m.stack.runtime, None);
    assert_eq!(m.project.description, None);
}

// --- archived is orthogonal to status (ADR-0002 §6) ---------------------

#[test]
fn shipped_and_archived_are_distinct_from_dead_and_archived() {
    let shipped = r#"schema_version = "1.0"
[project]
name = "done-and-filed"
status = "shipped"
archived = true
"#;
    let dead = r#"schema_version = "1.0"
[project]
name = "abandoned-and-filed"
status = "dead"
archived = true
"#;

    let s = doc(shipped).parse().unwrap();
    let d = doc(dead).parse().unwrap();

    assert_eq!(s.project.status, Status::Shipped);
    assert!(s.project.archived);
    assert_eq!(d.project.status, Status::Dead);
    assert!(d.project.archived);
    assert_ne!(s.project.status, d.project.status);

    // Both must survive a read/write cycle unchanged.
    for text in [shipped, dead] {
        assert_eq!(doc(text).render(), text);
    }
}

#[test]
fn archiving_never_touches_status_so_unarchiving_is_not_a_guess() {
    let original = r#"schema_version = "1.0"
[project]
name = "thing"
status = "shipped"
"#;

    let mut d = doc(original);
    d.apply(FieldEdit::SetArchived(true)).unwrap();
    let archived = d.render();
    assert!(archived.contains(r#"status = "shipped""#));
    assert!(archived.contains("archived = true"));

    // Un-archive restores exactly the prior state, because archiving never
    // overwrote it.
    let mut d = doc(&archived);
    d.apply(FieldEdit::SetArchived(false)).unwrap();
    let restored = d.parse().unwrap();
    assert_eq!(restored.project.status, Status::Shipped);
    assert!(!restored.project.archived);
}

#[test]
fn archiving_an_already_archived_project_is_a_no_op_not_an_error() {
    let text = r#"schema_version = "1.0"
[project]
name = "thing"
status = "paused"
archived = true
"#;
    let mut d = doc(text);
    d.apply(FieldEdit::SetArchived(true)).unwrap();
    // No change to write means no op at all — which is what keeps `--dry-run`
    // from showing an empty diff and `archive` from failing on a repeat.
    assert!(
        d.into_op(vibe_core::manifest::EditReason::FieldsUpdated)
            .is_none()
    );
}

#[test]
fn absent_archived_reads_as_false_so_old_manifests_are_unaffected() {
    let m = doc(r#"schema_version = "1.0"
[project]
name = "thing"
"#)
    .parse()
    .unwrap();
    assert!(!m.project.archived);
}

// --- schema version rules (ADR-0002 §3-§4) ------------------------------

#[test]
fn a_manifest_with_no_schema_version_is_read_as_one_zero() {
    let m = doc("[project]\nname = \"legacy\"\n").parse().unwrap();
    assert_eq!(m.schema_version, SchemaVersion::new(1, 0));
}

#[test]
fn a_newer_minor_is_read_and_its_unknown_keys_are_reported() {
    let m = doc(r#"schema_version = "1.7"
[project]
name = "from-the-future"
[deploy]
regions = ["fra1"]
"#)
    .parse()
    .expect("a newer minor must still be readable");

    assert_eq!(m.schema_version.compat(), Compat::MinorNewer);
    assert!(m.schema_version.compat().is_writable());
    assert!(m.unknown.iter().any(|u| u.dotted_path == "deploy.regions"));
}

#[test]
fn a_newer_major_is_refused_with_both_numbers_named() {
    let err = doc(r#"schema_version = "3.0"
[project]
name = "way-ahead"
"#)
    .parse()
    .expect_err("a newer major must be refused");

    assert_eq!(err.code(), "VIBE_E_SCHEMA_MAJOR_MISMATCH");
    let wire = err.to_wire();
    assert_eq!(wire.params["found"], "3.0");
    assert_eq!(wire.params["supported_major"], "1");
}

#[test]
fn an_older_major_is_also_refused() {
    let err = doc("schema_version = \"0.4\"\n[project]\nname = \"ancient\"\n")
        .parse()
        .expect_err("a different major in either direction is refused");
    assert_eq!(err.code(), "VIBE_E_SCHEMA_MAJOR_MISMATCH");
}

#[test]
fn an_unreadable_schema_version_says_so_rather_than_guessing() {
    for bad in ["\"banana\"", "1.0", "2", "true"] {
        let err = doc(&format!(
            "schema_version = {bad}\n[project]\nname = \"x\"\n"
        ))
        .parse()
        .expect_err("`{bad}` should not be accepted");
        assert_eq!(
            err.code(),
            "VIBE_E_SCHEMA_UNREADABLE",
            "unexpected error for {bad}: {err}"
        );
    }
}

// --- tolerance vs. honesty ----------------------------------------------

#[test]
fn an_unrecognised_status_is_kept_verbatim_not_normalised() {
    let text = r#"schema_version = "1.0"
[project]
name = "thing"
status = "hibernating"
"#;
    let m = doc(text).parse().unwrap();
    assert_eq!(m.project.status, Status::Other("hibernating".to_owned()));
    assert!(!m.project.status.is_known());
    // Untouched on write.
    assert_eq!(doc(text).render(), text);
}

#[test]
fn a_known_key_with_the_wrong_type_is_an_error_not_a_guess() {
    let err = doc(r#"schema_version = "1.0"
[project]
name = "thing"
[stack]
runtime = 42
"#)
    .parse()
    .expect_err("a wrong-typed known key is a broken file");

    assert_eq!(err.code(), "VIBE_E_MANIFEST_FIELD_TYPE");
    let wire = err.to_wire();
    assert_eq!(wire.params["field"], "stack.runtime");
    assert_eq!(wire.params["expected"], "a string");
    assert_eq!(wire.params["found"], "an integer");
}

#[test]
fn a_manifest_without_a_name_is_refused() {
    let err = doc("schema_version = \"1.0\"\n[project]\nstatus = \"active\"\n")
        .parse()
        .expect_err("a project with no name has no identity");
    assert_eq!(err.code(), "VIBE_E_MANIFEST_MISSING_FIELD");
    assert_eq!(err.to_wire().params["field"], "project.name");
}

#[test]
fn a_syntactically_broken_manifest_reports_syntax_not_something_vaguer() {
    let err = ManifestDocument::from_text(PATH, "[project\nname = ").expect_err("this is not TOML");
    assert!(matches!(err, CoreError::ManifestSyntax { .. }));
    assert_eq!(err.code(), "VIBE_E_MANIFEST_SYNTAX");
}

#[test]
fn editing_a_missing_table_creates_it_without_disturbing_the_rest() {
    let original = r#"# top
schema_version = "1.0"

[project]
name = "thing"  # keep me
"#;
    let mut d = doc(original);
    d.apply(FieldEdit::ReplaceDeployEnvRequired(vec![
        "SUPABASE_URL".to_owned(),
        "SUPABASE_ANON_KEY".to_owned(),
    ]))
    .unwrap();
    let after = d.render();

    assert!(after.contains("# top"));
    assert!(after.contains("# keep me"));
    assert!(after.contains("SUPABASE_URL"));
    assert!(after.contains("SUPABASE_ANON_KEY"));
    assert_eq!(
        doc(&after).parse().unwrap().deploy.env_required,
        vec!["SUPABASE_URL", "SUPABASE_ANON_KEY"]
    );
}
