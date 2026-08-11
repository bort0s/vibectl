//! `vibe render` through the binary.
//!
//! The assertion this file exists for is that **a hand-written `README.md`
//! survives `--force`**. Everything else here is supporting cast.
//!
//! Controls are paired per ADR-0002 §7: each refusal is asserted against the
//! matching case where the command *must* proceed. A test that only checks
//! refusal passes equally against a `render` that never renders anything, which
//! is a different bug wearing the same output.

use std::path::{Path, PathBuf};
use std::process::Command;

fn vibe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vibe"))
}

fn project(dir: &Path) -> PathBuf {
    let proj = dir.join("proj");
    std::fs::create_dir_all(proj.join(".vibe")).expect("mkdir");
    std::fs::write(
        proj.join(".vibe/project.toml"),
        "schema_version = \"1.1\"\n\n[project]\nname = \"macroring\"\n\
         description = \"Mobile-first PWA for nutrition tracking\"\nstatus = \"active\"\n\n\
         [stack]\nruntime = \"node@22\"\nframeworks = [\"react@19\"]\n",
    )
    .expect("write manifest");
    proj
}

fn render(proj: &Path, target: &str, extra: &[&str]) -> std::process::Output {
    vibe()
        .args(["render", target, "--path"])
        .arg(proj)
        .args(extra)
        .output()
        .expect("run vibe")
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn render_generates_the_file_and_a_rerun_is_a_no_op() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = project(tmp.path());

    let out = render(&proj, "claude", &[]);
    assert!(out.status.success(), "{}", stderr(&out));

    let text = std::fs::read_to_string(proj.join("CLAUDE.md")).expect("CLAUDE.md");
    assert!(
        text.starts_with("<!-- vibe:generated v1 hash=b3:"),
        "{text}"
    );
    assert!(text.contains("macroring"), "{text}");
    assert!(text.contains("node@22"), "{text}");

    // Second run: byte-identical, so nothing is written and it says so rather
    // than showing an empty diff.
    let again = render(&proj, "claude", &[]);
    assert!(again.status.success());
    assert!(
        String::from_utf8_lossy(&again.stdout).contains("already up to date"),
        "{}",
        String::from_utf8_lossy(&again.stdout)
    );
    assert_eq!(
        std::fs::read_to_string(proj.join("CLAUDE.md")).unwrap(),
        text,
        "a no-op run rewrote the file"
    );
}

/// **The reason `README.md` is allowed to be a target at all.**
///
/// Paired: the same command that must refuse a hand-written README must render
/// one into a project that has none.
#[test]
fn a_hand_written_readme_survives_even_force() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = project(tmp.path());

    let precious = "# macroring\n\nEight months of prose I wrote by hand.\n";
    std::fs::write(proj.join("README.md"), precious).unwrap();

    for flags in [vec![], vec!["--force"]] {
        let out = render(&proj, "readme", &flags);
        assert_eq!(
            out.status.code(),
            Some(1),
            "render {flags:?} should have refused: {}",
            stderr(&out)
        );
        assert_eq!(
            std::fs::read_to_string(proj.join("README.md")).unwrap(),
            precious,
            "render {flags:?} destroyed a hand-written README"
        );
    }
    // And the message says --force will not help, rather than suggesting it.
    let err = stderr(&render(&proj, "readme", &["--force"]));
    assert!(err.contains("not even with --force"), "{err}");

    // The paired half: with no README present, it renders.
    std::fs::remove_file(proj.join("README.md")).unwrap();
    let out = render(&proj, "readme", &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(proj.join("README.md").exists());
}

/// An edited generated file refuses, and `--force` is the way through — the
/// opposite of the `Foreign` case above, which is the whole distinction.
#[test]
fn an_edited_generated_file_refuses_until_force() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = project(tmp.path());
    assert!(render(&proj, "claude", &[]).status.success());

    let path = proj.join("CLAUDE.md");
    let edited = format!(
        "{}\nA paragraph I added by hand.\n",
        std::fs::read_to_string(&path).unwrap()
    );
    std::fs::write(&path, &edited).unwrap();

    let out = render(&proj, "claude", &[]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        edited,
        "the edit was destroyed without --force"
    );
    let err = stderr(&out);
    assert!(
        err.contains("--force"),
        "the way through must be named: {err}"
    );
    assert!(!err.contains("not even with --force"), "{err}");

    // Paired: --force does go through, and this is the direction that separates
    // Modified from Foreign.
    let forced = render(&proj, "claude", &["--force"]);
    assert!(forced.status.success(), "{}", stderr(&forced));
    assert!(!std::fs::read_to_string(&path).unwrap().contains("by hand"));
}

#[test]
fn dry_run_writes_absolutely_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = project(tmp.path());

    let out = render(&proj, "agents", &["--dry-run"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        !proj.join("AGENTS.md").exists(),
        "--dry-run created the file"
    );
    // It still showed the content it would have written, or the dry run is
    // useless.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("macroring"), "{stdout}");
    assert!(stderr(&out).contains("dry run"));
}

#[test]
fn an_unknown_target_is_rejected_and_names_the_valid_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = project(tmp.path());

    let out = render(&proj, "../../etc/passwd", &[]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("claude") && err.contains("readme"), "{err}");
    // Nothing was written anywhere.
    assert!(!proj.join("passwd").exists());
}

#[test]
fn json_output_is_parseable_and_is_the_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = project(tmp.path());

    let out = render(&proj, "claude", &["--dry-run", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let value: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("pure JSON on stdout");
    assert_eq!(value["intent"], "render");
    assert_eq!(value["ops"][0]["op"], "create_file");
    assert!(
        value["ops"][0]["contents"]
            .as_str()
            .unwrap()
            .contains("vibe:generated"),
        "the plan must carry what would be written"
    );
}
