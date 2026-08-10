//! The constraint this tool lives or dies by: **when the stack cannot be
//! inferred, write an empty field and flag it — never guess, never invent a
//! plausible-looking value.**
//!
//! Every fixture here is a directory where a plausible-looking wrong answer
//! would be *easy* and would look better in a demo than the honest empty one.
//! That is the point. A detection suite made only of well-formed projects
//! proves the happy path and says nothing about the failure this constraint
//! exists to prevent.
//!
//! Each test asserts two things: that the honest answer is produced, and that
//! the specific tempting wrong answer is *not*.

use std::path::Path;
use std::sync::Arc;

use vibe_core::model::UnknownReason;
use vibe_core::{Config, Detected, NoRunner, NullReporter, Registry, ScanRequest, ScannedProject};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("has a parent")).expect("mkdir");
    std::fs::write(p, contents).expect("write");
}

/// Scan a directory containing one project, with `git` unavailable unless the
/// test asks for it.
fn scan_one(root: &Path, with_git: bool) -> Option<ScannedProject> {
    let registry = if with_git {
        Registry::open(Config::default())
    } else {
        Registry::open(Config::default()).with_runner(Arc::new(NoRunner))
    };
    let report = registry.scan(&ScanRequest::new(root), &NullReporter);
    report.projects.into_iter().next()
}

fn assert_no_value<T: std::fmt::Debug>(d: &Detected<T>, what: &str) {
    assert!(
        !d.is_known(),
        "{what} should have no value, got {:?}",
        d.value()
    );
}

// --- nothing there at all ------------------------------------------------

#[test]
fn an_empty_directory_is_not_a_project_at_all() {
    let dir = tempfile::tempdir().expect("tempdir");
    let registry = Registry::open(Config::default()).with_runner(Arc::new(NoRunner));
    let report = registry.scan(&ScanRequest::new(dir.path()), &NullReporter);

    // Not "a project with everything unknown" — not a project. Inventing a
    // registry entry for an empty directory would fill a user's list with
    // noise they then have to archive one by one.
    assert!(
        report.projects.is_empty(),
        "an empty directory must not become a registry entry, got {:?}",
        report.projects
    );
}

#[test]
fn a_directory_with_only_a_readme_is_not_a_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        dir.path(),
        "notes/README.md",
        "# Some ideas\n\nMaybe a Rust CLI. Or Python.\n",
    );

    let registry = Registry::open(Config::default()).with_runner(Arc::new(NoRunner));
    let report = registry.scan(&ScanRequest::new(dir.path()), &NullReporter);

    // The README *mentions* Rust and Python. A tool that reads prose for stack
    // hints would claim one of them here.
    assert!(
        report.projects.is_empty(),
        "prose in a README is not evidence of a stack"
    );
}

// --- present but uninformative -------------------------------------------

#[test]
fn a_package_json_with_no_dependencies_yields_no_version_and_no_frameworks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("bare");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(
        &root,
        "package.json",
        r#"{ "name": "bare", "version": "0.1.0" }"#,
    );

    let p = scan_one(dir.path(), false).expect("bare is a project");

    // The runtime is claimed, because a package.json genuinely proves Node —
    // and the evidence cites the file's existence, not an inference.
    let runtime = p.detection.runtime.value().map(String::as_str);
    assert_eq!(runtime, Some("node"));

    // The honest part: no version is invented. There is no `engines` field, no
    // lockfile, and nothing else that states one.
    assert!(
        !runtime.expect("known").contains('@'),
        "no version may be invented from a package.json without engines, got {runtime:?}"
    );
    assert!(
        p.detection.frameworks.is_empty(),
        "no dependencies, no frameworks"
    );
    assert!(p.detection.services.is_empty());
    assert_no_value(&p.detection.description, "description");
    assert!(
        p.detection.suggestions.is_empty(),
        "nothing to suggest either: {:?}",
        p.detection.suggestions
    );
}

#[test]
fn an_engines_range_of_star_yields_no_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("loose");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(
        &root,
        "package.json",
        r#"{ "name": "loose", "engines": { "node": "*" } }"#,
    );

    let p = scan_one(dir.path(), false).expect("project");
    // "*" is a range with no floor. Reading it as "the current LTS" or as the
    // machine's installed Node would both be inventions.
    assert_eq!(
        p.detection.runtime.value().map(String::as_str),
        Some("node")
    );
}

// --- present but broken ---------------------------------------------------

#[test]
fn a_malformed_cargo_toml_is_reported_unreadable_not_as_no_evidence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("broken");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(&root, "Cargo.toml", "[package\nname = \"oops\"\n");

    let p = scan_one(dir.path(), false).expect("broken is still a project");

    assert_no_value(&p.detection.runtime, "runtime");

    // The distinction that matters. "No evidence" is a clean bill of health;
    // "unreadable" is something the user can go and fix. Presenting the second
    // as the first is a quiet lie.
    let reason = p.detection.runtime.reason().expect("unknown has a reason");
    let UnknownReason::Unreadable { path, why } = reason else {
        panic!("expected Unreadable, got {reason:?}");
    };
    assert_eq!(path.to_string_lossy(), "Cargo.toml");
    assert!(!why.is_empty(), "the parse error should be carried");

    // And the tempting wrong answer — "it has a Cargo.toml, so it's Rust" —
    // is not produced.
    assert!(
        p.detection.runtime.value().is_none(),
        "a file we could not parse must not yield a stack claim"
    );
}

#[test]
fn a_malformed_package_json_is_reported_unreadable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("broken");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(&root, "package.json", "{ \"name\": \"oops\", }");

    let p = scan_one(dir.path(), false).expect("project");
    assert_no_value(&p.detection.runtime, "runtime");
    assert!(matches!(
        p.detection.runtime.reason(),
        Some(UnknownReason::Unreadable { .. })
    ));
}

// --- git with nothing to say ----------------------------------------------

fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_fresh_repo_with_no_remote_and_no_commits_claims_neither() {
    if !git_available() {
        eprintln!("skipping: git is not on PATH");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("fresh");
    std::fs::create_dir_all(&root).expect("mkdir");
    let ok = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "git init should succeed");

    let p = scan_one(dir.path(), true).expect("a git repo is a project");

    // Every one of these has an obvious wrong answer available: "origin",
    // "main", and "now".
    assert_no_value(&p.detection.remote, "remote");
    assert_no_value(&p.detection.last_commit, "last commit");
    assert_eq!(
        p.detection.remote.reason(),
        Some(&UnknownReason::NoEvidence),
        "a repo with no remote has no remote — not a guessed one"
    );
    assert!(
        p.detection.suggestions.is_empty(),
        "nothing should be suggested either: {:?}",
        p.detection.suggestions
    );
}

#[test]
fn without_git_installed_repo_fields_say_so_rather_than_appearing_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("repo");
    std::fs::create_dir_all(root.join(".git")).expect("mkdir");
    write(&root, ".git/config", "[core]\n");

    // NoRunner is exactly the code path of a user without git installed.
    let p = scan_one(dir.path(), false).expect("project");

    assert_no_value(&p.detection.remote, "remote");
    // Not NoEvidence: there is a repository here and we could not ask about it.
    // Reporting that as "no remote" would tell the user their repo has no
    // remote, which may be false.
    let reason = p.detection.remote.reason().expect("has a reason");
    assert!(
        matches!(reason, UnknownReason::NotAttempted { .. }),
        "expected NotAttempted, got {reason:?}"
    );
}

// --- the seductive cases --------------------------------------------------

#[test]
fn a_dockerfile_mentioning_a_language_is_not_evidence_of_the_stack() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("dockerish");
    std::fs::create_dir_all(&root).expect("mkdir");
    // A project marker so it is scanned at all, with no stack information.
    std::fs::create_dir_all(root.join(".git")).expect("mkdir");
    write(&root, "Dockerfile", "FROM python:3.11-slim\nCOPY . /app\n");
    write(
        &root,
        "docker-compose.yml",
        "services:\n  web:\n    build: .\n",
    );

    let p = scan_one(dir.path(), false).expect("project");

    // The 11pm case from ADR-0003. It is *obviously* Python to a human. There
    // is no pyproject.toml, no requirements.txt, and no .py file, so the tool
    // says nothing.
    assert_no_value(&p.detection.runtime, "runtime");
    assert!(
        p.detection
            .suggestions
            .iter()
            .all(|s| !s.value.contains("python")),
        "not even as a suggestion, since no detector examined the Dockerfile: {:?}",
        p.detection.suggestions
    );
}

#[test]
fn a_polyglot_repo_gets_an_empty_runtime_and_a_reported_conflict() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("polyglot");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(
        &root,
        "package.json",
        r#"{"name":"web","engines":{"node":">=22"}}"#,
    );
    write(&root, "go.mod", "module example.com/api\n\ngo 1.23\n");

    let p = scan_one(dir.path(), false).expect("project");

    // Picking one would look better and be a lie. Both are Certain-or-better
    // claims from authoritative manifests, and the ladder refuses to choose.
    assert_no_value(&p.detection.runtime, "runtime");
    let Some(UnknownReason::Conflict { candidates }) = p.detection.runtime.reason() else {
        panic!(
            "expected a conflict, got {:?}",
            p.detection.runtime.reason()
        );
    };
    let values: Vec<&str> = candidates.iter().map(|c| c.value.as_str()).collect();
    assert!(values.contains(&"go@1.23"), "got {values:?}");

    // Set-valued fields still work, so the entry is not useless.
    assert!(
        !p.detection.suggestions.is_empty(),
        "both candidates should be reported so a human can resolve it"
    );
}

#[test]
fn a_netlify_project_gets_no_invented_url() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("sitename");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(&root, "package.json", "{}");
    write(
        &root,
        "netlify.toml",
        "[build]\n  command = \"npm run build\"\n",
    );

    let p = scan_one(dir.path(), false).expect("project");

    assert!(
        p.detection.services.contains(&"netlify".to_owned()),
        "the service is provable"
    );
    // `https://sitename.netlify.app` is right often enough to be tempting and
    // wrong often enough to be a liability. netlify.toml does not record the
    // site name, so nothing is claimed.
    assert_no_value(&p.detection.deploy_url, "deploy url");
}

#[test]
fn a_secret_bearing_env_file_is_never_read_even_for_key_names() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("secretive");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(&root, "package.json", "{}");
    write(&root, ".env", "SUPABASE_SERVICE_KEY=super-secret-value\n");
    write(&root, ".env.example", "SUPABASE_URL=\nSUPABASE_ANON_KEY=\n");

    let p = scan_one(dir.path(), false).expect("project");

    // Key names from the example file: fine, and the point of the feature.
    assert_eq!(
        p.detection.env_required,
        vec!["SUPABASE_ANON_KEY", "SUPABASE_URL"]
    );

    // Nothing from the real .env — neither its key names nor, obviously, its
    // values. The whole report is checked, because a leak into an evidence
    // excerpt would be just as bad as one into a field.
    let serialized = serde_json::to_string(&p).expect("serialises");
    assert!(
        !serialized.contains("super-secret-value"),
        "a secret value reached the scan output"
    );
    assert!(
        !serialized.contains("SUPABASE_SERVICE_KEY"),
        "a key name from the real .env reached the scan output"
    );
}

// --- determinism ----------------------------------------------------------

#[test]
fn two_scans_of_an_unchanged_tree_are_identical() {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["alpha", "beta", "gamma"] {
        let root = dir.path().join(name);
        std::fs::create_dir_all(&root).expect("mkdir");
        write(
            &root,
            "package.json",
            r#"{"name":"x","dependencies":{"react":"^19.0.0","vite":"^5.0.0"}}"#,
        );
        write(&root, ".env.example", "A=\nB=\n");
    }

    let registry = Registry::open(Config::default()).with_runner(Arc::new(NoRunner));
    let one = registry.scan(&ScanRequest::new(dir.path()), &NullReporter);
    let two = registry.scan(&ScanRequest::new(dir.path()), &NullReporter);

    // Timing differs; everything else must not. Two scans that disagree would
    // make `vibe sync` produce a diff on every run.
    assert_eq!(one.projects, two.projects);
}

// --- regressions from the adversarial review ----------------------------
//
// Every case below produced a confident wrong answer, a fabricated citation,
// or an invented failure before it was fixed. They are grouped here because
// they share one root cause: a detector's *interest* matched something its
// *read* never touched.

#[test]
fn a_monorepo_does_not_invent_a_root_manifest_it_never_had() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("mono");
    std::fs::create_dir_all(root.join(".git")).expect("mkdir");
    write(
        &root,
        "web/package.json",
        r#"{"name":"web","engines":{"node":">=22"},"dependencies":{"react":"^19.0.0"}}"#,
    );
    write(&root, "api/go.mod", "module example.com/api\n\ngo 1.23\n");

    let p = scan_one(dir.path(), false).expect("project");

    // Previously: Unknown{Unreadable, path: "package.json"} — a specific,
    // actionable-looking failure about a file that never existed. Reporting a
    // fabricated read error sends the user hunting for a corrupt file.
    let reason = p.detection.runtime.reason();
    assert!(
        !matches!(reason, Some(UnknownReason::Unreadable { .. })),
        "a manifest in a subdirectory must not produce a root read error: {reason:?}"
    );
    assert!(
        p.detection.unreadable.is_empty(),
        "nothing was unreadable here: {:?}",
        p.detection.unreadable
    );
}

#[test]
fn a_gitignored_vercel_directory_is_still_seen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("shop");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(&root, "package.json", r#"{"name":"shop"}"#);
    write(&root, ".vercel/project.json", r#"{"name":"my-shop"}"#);
    // `vercel link` writes exactly this.
    write(&root, ".gitignore", ".vercel\nnode_modules\n");

    let p = scan_one(dir.path(), false).expect("project");

    // Honouring .gitignore made the deploy detectors blind on precisely the
    // projects that are deployed, and reported it as `no_evidence` — a claim
    // that the disk was silent when it was not.
    assert!(
        p.detection.services.contains(&"vercel".to_owned()),
        "services: {:?}",
        p.detection.services
    );
    assert_eq!(
        p.detection.deploy_url.value().map(String::as_str),
        Some("https://my-shop.vercel.app")
    );
}

#[test]
fn a_nested_lockfile_does_not_become_a_service_of_the_parent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("engine");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(&root, "Cargo.toml", "[package]\nname = \"engine\"\n");
    write(
        &root,
        "examples/wasm-demo/package.json",
        r#"{"name":"wasm-demo","dependencies":{"vite":"^5.0.0"}}"#,
    );
    write(
        &root,
        "examples/wasm-demo/package-lock.json",
        r#"{"lockfileVersion":3}"#,
    );

    let p = scan_one(dir.path(), false).expect("project");

    assert_eq!(
        p.detection.runtime.value().map(String::as_str),
        Some("rust")
    );
    assert!(
        !p.detection.services.contains(&"npm".to_owned()),
        "an example's lockfile is not a service of the crate: {:?}",
        p.detection.services
    );
    assert!(
        !p.detection.frameworks.contains(&"vite@5".to_owned()),
        "nor are its dependencies the crate's frameworks: {:?}",
        p.detection.frameworks
    );
}

#[test]
fn docs_requirements_txt_does_not_make_a_go_project_polyglot() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("srv");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(&root, "go.mod", "module example.com/srv\n\ngo 1.23\n");
    // Every project with Sphinx or MkDocs docs has this file.
    write(&root, "docs/requirements.txt", "sphinx==7.2.6\n");

    let p = scan_one(dir.path(), false).expect("project");
    assert_eq!(
        p.detection.runtime.value().map(String::as_str),
        Some("go@1.23"),
        "a docs build dependency is not the project's runtime"
    );
}

#[test]
fn tool_config_tables_are_not_read_as_dependencies() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("acme");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(
        &root,
        "pyproject.toml",
        r#"[project]
name = "acme-cli"
requires-python = ">=3.9"
dependencies = ["click"]

[tool.pytest.ini_options]
testpaths = ["tests"]

[tool.mypy]
plugins = ["pydantic.mypy"]
"#,
    );

    let p = scan_one(dir.path(), false).expect("project");

    // A substring scan over the whole file claimed both, and cited both to
    // `project.dependencies` — a table containing neither.
    assert!(
        p.detection.frameworks.is_empty(),
        "only `click` is a dependency, and it is not in the framework list: {:?}",
        p.detection.frameworks
    );
}

#[test]
fn breaking_a_manifest_never_raises_confidence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("svc");
    std::fs::create_dir_all(&root).expect("mkdir");
    write(
        &root,
        "package.json",
        r#"{"name":"svc","engines":{"node":">=20"}}"#,
    );
    write(
        &root,
        "pyproject.toml",
        "[project\nname = \"svc\"\nrequires-python = \">=3.12\"\n",
    );

    let p = scan_one(dir.path(), false).expect("project");

    // The value may still resolve — one readable manifest is real evidence.
    // What must not happen is the failure vanishing: with the pyproject
    // readable this is a conflict, so corrupting it must not silently produce
    // a confident single answer with nothing to show for the loss.
    assert!(
        !p.detection.unreadable.is_empty(),
        "the unreadable pyproject.toml must be reported even though runtime resolved"
    );
    let u = &p.detection.unreadable[0];
    assert_eq!(u.path, "pyproject.toml");
    assert!(
        u.affects.contains(&vibe_core::FieldPath::StackRuntime),
        "and it must say which fields it leaves less certain: {:?}",
        u.affects
    );
}
