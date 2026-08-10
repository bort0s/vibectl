//! End-to-end coverage of the binary itself: exit codes, what lands on disk,
//! and the one thing `--dry-run` must never do.

use std::path::Path;
use std::process::Command;

fn vibe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vibe"))
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn manifest_in(dir: &Path, name: &str) -> std::path::PathBuf {
    dir.join(name).join(".vibe").join("project.toml")
}

#[test]
fn dry_run_writes_absolutely_nothing() {
    let dir = tmp();

    let out = vibe()
        .args(["new", "macroring", "--dry-run", "--path"])
        .arg(dir.path())
        .output()
        .expect("run vibe");

    assert!(out.status.success(), "dry run should succeed");

    // The whole contract of --dry-run, asserted directly rather than inferred
    // from the absence of an error.
    assert!(
        !dir.path().join("macroring").exists(),
        "--dry-run created a directory"
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "--dry-run left something behind"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("create dir"), "should describe the plan");
    assert!(
        stdout.contains("schema_version"),
        "should show the manifest it would write"
    );
    // Progress and disclaimers go to stderr so stdout stays pipeable.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("dry run"),
        "the disclaimer belongs on stderr"
    );
}

#[test]
fn dry_run_json_is_valid_json_on_stdout_only() {
    let dir = tmp();

    let out = vibe()
        .args(["new", "thing", "--dry-run", "--json", "--path"])
        .arg(dir.path())
        .output()
        .expect("run vibe");

    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be parseable JSON on its own");

    assert_eq!(parsed["intent"], "new");
    assert_eq!(parsed["ops"].as_array().unwrap().len(), 3);
    assert_eq!(parsed["ops"][0]["op"], "create_dir");
    assert_eq!(parsed["ops"][2]["op"], "create_file");

    // Windows extended-length prefixes are an implementation detail of
    // `canonicalize` and must not reach output a human or a script reads.
    let root = parsed["root"].as_str().unwrap();
    assert!(
        !root.starts_with(r"\\?\"),
        "extended-length path leaked into --json: {root}"
    );
}

#[test]
fn new_creates_a_manifest_that_reads_back() {
    let dir = tmp();

    let out = vibe()
        .args([
            "new",
            "macroring",
            "--description",
            "Mobile-first PWA for nutrition tracking",
            "--status",
            "paused",
            "--path",
        ])
        .arg(dir.path())
        .output()
        .expect("run vibe");

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let manifest = manifest_in(dir.path(), "macroring");
    let text = std::fs::read_to_string(&manifest).expect("manifest should exist");

    assert!(text.contains(r#"name = "macroring""#));
    assert!(text.contains(r#"status = "paused""#));
    assert!(text.contains("Mobile-first PWA for nutrition tracking"));
    assert!(text.contains(r#"schema_version = "1.0""#));
    // The generated header survives, because it is part of the document rather
    // than a string prepended at write time.
    assert!(text.contains("# Managed by vibe"));
}

#[test]
fn refusing_to_adopt_an_existing_directory_exits_one_with_a_hint() {
    let dir = tmp();
    std::fs::create_dir(dir.path().join("taken")).unwrap();

    let out = vibe()
        .args(["new", "taken", "--path"])
        .arg(dir.path())
        .output()
        .expect("run vibe");

    assert_eq!(out.status.code(), Some(1), "failure is exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("already exists"));
    assert!(
        stderr.contains("vibe scan"),
        "the hint should point at the command that does adopt directories"
    );
    assert!(out.stdout.is_empty(), "errors must not go to stdout");
}

#[test]
fn an_unknown_status_is_accepted_rather_than_rejected_at_the_cli() {
    let dir = tmp();

    // The file format tolerates a status this build does not know, so the CLI
    // must not be stricter than the format it writes.
    let out = vibe()
        .args(["new", "thing", "--status", "hibernating", "--path"])
        .arg(dir.path())
        .output()
        .expect("run vibe");

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(manifest_in(dir.path(), "thing")).unwrap();
    assert!(text.contains(r#"status = "hibernating""#));
}

#[test]
fn paths_in_output_are_readable_not_extended_length() {
    let dir = tmp();

    let out = vibe()
        .args(["new", "thing", "--dry-run", "--path"])
        .arg(dir.path())
        .output()
        .expect("run vibe");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(r"\\?\"),
        "extended-length path leaked into human output:\n{stdout}"
    );
}
