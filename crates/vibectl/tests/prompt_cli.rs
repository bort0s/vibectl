//! `vibe prompt` through the binary.
//!
//! The renderer's own controls are unit tests in `src/prompts/tests.rs`, where a
//! scripted runner buys all four exposure states deterministically. **This file
//! covers the wiring those cannot reach**: that the subcommands exist, that the
//! exit contract holds, and that `show` puts the file on stdout unaltered.
//!
//! # Deliberately not gated on a tool being present
//!
//! Nothing here asserts an exposure *state*, so nothing here needs `git`. That
//! is a scoping decision rather than an oversight: the states are established in
//! `vibe-core`'s `prompts_listing.rs`, against the real instrument and under the
//! guard that turns a missing git into a failure instead of a skip. Repeating
//! them here would add a second, weaker copy of a control that already exists —
//! and would add a target to the inventory gate for no coverage.
//!
//! Every assertion below therefore holds on a machine with no `git` at all,
//! where every project row simply reads `unknown`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn vibe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vibe"))
}

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("has a parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write");
}

fn a_project(root: &Path) -> PathBuf {
    write(
        &root.join(".claude/commands/daily.md"),
        "the daily prompt\n",
    );
    write(
        &root.join(".claude/commands/shared/deploy.md"),
        "---\nmodel: haiku\n---\n\nship it `now` $(please)\n",
    );
    root.to_path_buf()
}

fn run(args: &[&str]) -> std::process::Output {
    vibe().args(args).output().expect("run vibe")
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn prompt_list_shows_the_derived_names_and_an_exposure_column() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let proj = a_project(tmp.path());

    let out = run(&["prompt", "list", &proj.display().to_string()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = stdout(&out);

    // The measured name mapping reaches the screen: a separator becomes `:`.
    assert!(text.contains("daily"), "{text}");
    assert!(text.contains("shared:deploy"), "{text}");
    assert!(text.contains("EXPOSURE"), "{text}");
    // §6's `NotAttempted`, on screen, unconditionally.
    assert!(
        text.contains("Plugin-supplied prompts were not checked"),
        "{text}"
    );
}

/// **The exit contract, both halves.**
///
/// `Partial` means *the listing is partial*. A complete listing exits `0`
/// whatever the exposure states turn out to be on this machine — which is what
/// keeps "a directory could not be read" distinguishable from "git could not
/// answer", the distinction ADR-0002 §3's exit codes exist for.
#[test]
fn an_unreadable_root_exits_partial_and_a_complete_one_does_not() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let ok = a_project(&tmp.path().join("ok"));
    let out = run(&["prompt", "list", &ok.display().to_string()]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));

    // `.claude/commands` is a file, so the root's own `read_dir` fails.
    let broken = tmp.path().join("broken");
    write(&broken.join(".claude/commands"), "not a directory\n");
    let out = run(&["prompt", "list", &broken.display().to_string()]);
    assert_eq!(out.status.code(), Some(2), "{}", stdout(&out));

    let text = stdout(&out);
    assert!(
        !text.contains("This project defines no prompts."),
        "an unreadable directory was reported as a project with no prompts\n{text}"
    );
}

/// A project that genuinely has none says so — the positive control on the
/// assertion above, without which it passes against a build that never prints
/// the sentence at all.
#[test]
fn a_project_with_no_prompts_says_so_and_exits_zero() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(empty.join(".claude/commands")).expect("mkdir");

    let out = run(&["prompt", "list", &empty.display().to_string()]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(stdout(&out).contains("This project defines no prompts."));
}

/// **`show` puts the file on stdout unaltered, frontmatter included** (§7).
///
/// The fixture's body is chosen to be hostile on purpose: backticks and `$(…)`
/// are the normal content of these files and are exactly what §9 refuses to put
/// into a shell string. A display that reformatted, escaped or stripped any of
/// it would show something other than what the model receives.
#[test]
fn prompt_show_prints_the_file_verbatim_including_frontmatter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let proj = a_project(tmp.path());

    let out = run(&[
        "prompt",
        "show",
        "shared:deploy",
        "--path",
        &proj.display().to_string(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = stdout(&out);

    let on_disk = std::fs::read_to_string(proj.join(".claude/commands/shared/deploy.md"))
        .expect("read the fixture");
    assert!(
        text.contains(&on_disk),
        "the body was altered on the way to the screen\nwanted:\n{on_disk}\ngot:\n{text}"
    );
    // Frontmatter specifically: it is stripped from what the model receives but
    // is not inert, so a display that hid it would hide the thing that changes
    // behaviour.
    assert!(text.contains("model: haiku"), "{text}");
    // §7's other two facts travel with it.
    assert!(text.contains("shared:deploy"), "{text}");
    assert!(text.contains("exposure"), "{text}");
}

/// A name nothing owns is a failure, and it is not silent about the difference
/// between *not found* and *does not exist*.
#[test]
fn prompt_show_fails_on_a_name_that_is_not_there() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let proj = a_project(tmp.path());

    let out = run(&[
        "prompt",
        "show",
        "nosuchprompt",
        "--path",
        &proj.display().to_string(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no prompt named `nosuchprompt`"), "{err}");
    // The complete-read case must NOT carry the incompleteness caveat, or it
    // would appear on every miss and mean nothing.
    assert!(!err.contains("rather than \"does not exist\""), "{err}");
}

/// `--json` is the Tauri frontend's path, so the shape it will key on is
/// asserted here rather than left to the first consumer to discover.
#[test]
fn prompt_list_json_carries_the_state_beside_the_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let proj = a_project(tmp.path());

    let out = run(&["prompt", "list", &proj.display().to_string(), "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("valid json");
    assert_eq!(json["plugins"]["plugins"], "not_attempted");

    let first = &json["prompts"][0];
    assert!(first["name"].is_string(), "{json}");
    // The pair, not a flattened label: the root is beside the state, which is
    // what makes the dotfiles opt-in additive (§5a).
    assert_eq!(first["exposure"]["root"]["root"], "project");
    assert!(
        first["exposure"]["state"]["state"].is_string(),
        "a project prompt carries an answer\n{json}"
    );
}
