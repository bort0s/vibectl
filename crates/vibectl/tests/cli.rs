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

// --- vibe scan -----------------------------------------------------------

fn make_project(root: &Path, name: &str, files: &[(&str, &str)]) {
    for (rel, body) in files {
        let p = root.join(name).join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, body).expect("write");
    }
}

#[test]
fn scan_indexes_projects_and_leaves_them_alone() {
    let dir = tmp();
    make_project(
        dir.path(),
        "macroring",
        &[(
            "package.json",
            r#"{"name":"macroring","description":"nutrition tracking","dependencies":{"react":"^19.0.0"}}"#,
        )],
    );
    make_project(
        dir.path(),
        "tideline",
        &[("Cargo.toml", "[package]\nname=\"tideline\"\n")],
    );

    let out = vibe()
        .args(["scan"])
        .arg(dir.path())
        .output()
        .expect("run vibe");

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("macroring"), "{stdout}");
    assert!(stdout.contains("tideline"), "{stdout}");
    assert!(stdout.contains("2 project(s)"), "{stdout}");

    // Scanning is a read. Nothing was created for either project.
    assert!(!dir.path().join("macroring/.vibe").exists());
    assert!(!dir.path().join("tideline/.vibe").exists());
}

#[test]
fn scan_json_is_parseable_and_marks_undetectable_fields_as_unknown() {
    let dir = tmp();
    // A Go project with no remote and no deploy config: several fields have
    // nothing to say, and each must say so explicitly.
    make_project(
        dir.path(),
        "solo",
        &[("go.mod", "module example.com/solo\n\ngo 1.23\n")],
    );

    let out = vibe()
        .args(["scan", "--json"])
        .arg(dir.path())
        .output()
        .expect("run vibe");

    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON on stdout");

    let project = &v["projects"][0];
    assert_eq!(project["name"], "solo");
    assert_eq!(project["detection"]["runtime"]["state"], "known");
    assert_eq!(project["detection"]["runtime"]["value"], "go@1.23");

    // The honest part, visible in the machine-readable output: an absent field
    // carries a reason, not an empty string that a script would read as data.
    assert_eq!(project["detection"]["deploy_url"]["state"], "unknown");
    assert_eq!(project["detection"]["deploy_url"]["reason"], "no_evidence");

    // Every claimed value cites its source.
    assert_eq!(
        project["detection"]["runtime"]["evidence"]["source"]["kind"], "file",
        "a value without a citation should be impossible"
    );
    assert_eq!(
        project["detection"]["runtime"]["evidence"]["source"]["path"], "go.mod",
        "the citation names the file it came from"
    );
    assert_eq!(
        project["detection"]["runtime"]["evidence"]["excerpt"],
        "go 1.23"
    );
}

#[test]
fn scanning_a_directory_with_no_projects_says_so_and_succeeds() {
    let dir = tmp();
    std::fs::write(dir.path().join("stray.txt"), "hello").expect("write");

    let out = vibe()
        .args(["scan"])
        .arg(dir.path())
        .output()
        .expect("run vibe");

    assert!(out.status.success(), "an empty result is not a failure");
    assert!(String::from_utf8_lossy(&out.stdout).contains("No projects found"));
}

#[test]
fn an_unreadable_manifest_exits_two_not_one() {
    let dir = tmp();
    make_project(
        dir.path(),
        "ahead",
        &[
            ("go.mod", "module example.com/ahead\n\ngo 1.23\n"),
            (
                ".vibe/project.toml",
                "schema_version = \"9.0\"\n[project]\nname=\"ahead\"\n",
            ),
        ],
    );
    make_project(
        dir.path(),
        "fine",
        &[("go.mod", "module example.com/fine\n\ngo 1.23\n")],
    );

    let out = vibe()
        .args(["scan"])
        .arg(dir.path())
        .output()
        .expect("run vibe");

    // The registry was read and one project could not be. That is a partial
    // result, and a script must be able to tell it from a total failure.
    assert_eq!(out.status.code(), Some(2), "expected the partial exit code");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("fine"),
        "the readable project still appears"
    );
    assert!(stdout.contains("unreadable"), "{stdout}");
}
