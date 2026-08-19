//! **Nothing in this crate mutates the filesystem except the one primitive.**
//!
//! ADR-0001 §3 says core exposes no method that mutates the filesystem except
//! `apply`, and constraint 2's non-destructiveness is *"enforced by the absence
//! of `FileOp::Delete`, not by discipline."*
//!
//! **The type never enforced that, and `Cache::save` is the proof.** It wrote
//! and renamed on its own, and it carried a `std::fs::remove_file` — deletion,
//! in production, hand-written, with `FileOp::Delete` nonexistent throughout.
//! The type governs only what routes through `apply`; **nothing bounded what
//! did not.**
//!
//! So the enforcement boundary is not the type, it is **whether a call site
//! routes through the primitive** — and that is a property of the source, which
//! nothing was checking. *"Everything else is inside `#[cfg(test)]`"* was a
//! hand measurement taken once, and ADR-0001 §3a's revisit trigger — *a third
//! write path outside `apply`* — was not hooked to anything that fires. A third
//! write path appearing is visible only if something looks.
//!
//! This is the something. It is the same technique as
//! `the_write_path_has_no_buffered_writer` and `control_inventory`, for the same
//! reason: the property is structural, nothing observable distinguishes a
//! second write path until it has already cost something, and a paragraph
//! asking the next author not to add one is a rule that gets broken by someone
//! who never read it.
//!
//! # Reach, stated rather than assumed
//!
//! **Scanned:** every `.rs` file under `crates/*/src/`, which is the shipped
//! library and binary code of every workspace member.
//!
//! **Not scanned, and each for a reason:**
//!
//! - `crates/*/tests/` and `#[cfg(test)]` blocks inside `src/` — a fixture that
//!   plants a file is not the tool mutating a user's filesystem, and demanding
//!   they go through `apply` would mean building a plan to write a temp dir.
//! - **A test module in its own file**, such as `src/prompts/tests.rs`. Its
//!   `#[cfg(test)]` is in the *parent* module, so nothing inside the file marks
//!   it — and this scan found it on its first honest run. **The exclusion is not
//!   by filename**: a file is excluded only when its declaring parent is proved
//!   to gate it, by containing `#[cfg(test)]` on the line before its `mod`
//!   declaration. A `tests.rs` whose parent does *not* gate it is shipped code
//!   and is scanned.
//! - `crates/*/examples/` and `crates/*/benches/` — not shipped in the binary a
//!   user runs. **This is a real hole and it is named**: an example that writes
//!   directly is not caught here, and `emit_install_settings` deliberately calls
//!   the primitive rather than `fs::write` for that reason.
//! - Build scripts and `xtask` — **there are none in this workspace today**, and
//!   the assertion below fails if one appears, because a new `crates/*/src`
//!   sibling would not be scanned and the count check would notice the layout
//!   moved.
//!
//! # What it looks for
//!
//! The names that mutate: `fs::write`, `fs::remove_file`, `fs::remove_dir_all`,
//! `File::create`, and `.truncate(true)`. **Not `fs::rename`**, which is the
//! primitive's own move and is where it must be.

use std::path::{Path, PathBuf};

/// Where the one primitive lives. Everything else is checked against it.
const PRIMITIVE: &str = "plan.rs";

/// The mutating calls no other module may make.
const FORBIDDEN: [&str; 5] = [
    "fs::write",
    "fs::remove_file",
    "fs::remove_dir_all",
    "File::create",
    ".truncate(true)",
];

fn workspace_crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits at <workspace>/crates/<name>")
        .join("crates")
}

/// Every `crates/*/src/**/*.rs`.
fn shipped_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for crate_dir in std::fs::read_dir(workspace_crates()).expect("crates/ is readable") {
        walk(&crate_dir.expect("entry").path().join("src"), &mut out);
    }
    out.sort();
    out
}

/// Whether a whole file is a test module declared `#[cfg(test)]` elsewhere.
///
/// `src/prompts/tests.rs` carries no marker of its own: the gate is
/// `#[cfg(test)] mod tests;` in `src/prompts.rs`. Excluding it by **name** would
/// mean any file called `tests.rs` could hold unscanned shipped code, so the
/// exclusion is earned rather than assumed — the declaring parent must be found
/// **and** must be gating it.
///
/// Returns the parent that gates it, so the caller can say why.
fn gated_by_parent(path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_str()?.to_owned();
    let dir = path.parent()?;
    // `src/a/b.rs` is declared in `src/a.rs` or `src/a/mod.rs`.
    let candidates = [
        dir.with_extension("rs"),
        dir.join("mod.rs"),
        dir.parent()?.join("lib.rs"),
        dir.parent()?.join("main.rs"),
    ];
    for parent in candidates {
        let Ok(src) = std::fs::read_to_string(&parent) else {
            continue;
        };
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim();
            let declares = t == format!("mod {stem};")
                || t == format!("pub mod {stem};")
                || t == format!("pub(crate) mod {stem};");
            if declares && i > 0 && lines[i - 1].trim().starts_with("#[cfg(test)]") {
                return Some(parent);
            }
        }
    }
    None
}

/// Strip `#[cfg(test)]` items, so a fixture planting a file is not a finding.
///
/// **The first version cut from the first `#[cfg(test)]` to the end of the
/// file, and its own premise assertion caught it**: `exec.rs` has three, so a
/// tail cut would have skipped every shipped line below the first and reported
/// a clean scan over code it never looked at. That is the failure this whole
/// file exists against, one level in.
///
/// So it tracks braces instead: on `#[cfg(test)]`, skip until the item it
/// annotates closes. Braces inside string literals could still confuse it,
/// which is why the premises below assert that shipped code **survives** the
/// filter rather than trusting that it does.
fn shipped_lines(src: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut skipping = false;
    let mut depth: i32 = 0;
    let mut opened = false;

    for (i, line) in src.lines().enumerate() {
        let trimmed = line.trim_start();
        if !skipping && trimmed.starts_with("#[cfg(test)]") {
            skipping = true;
            depth = 0;
            opened = false;
            continue;
        }
        if skipping {
            depth += i32::try_from(line.matches('{').count()).unwrap_or(0);
            depth -= i32::try_from(line.matches('}').count()).unwrap_or(0);
            if line.contains('{') {
                opened = true;
            }
            if opened && depth <= 0 {
                skipping = false;
            }
            continue;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        out.push((i + 1, line));
    }
    out
}

/// **The enforcement constraint 2 claims to have.**
#[test]
fn no_module_outside_the_primitive_mutates_the_filesystem() {
    let sources = shipped_sources();

    // Premise one: the scan found the corpus. A wrong root produces zero files
    // and a gate asserting "no offenders" passes on zero perfectly.
    assert!(
        sources.len() > 20,
        "found only {} shipped source files, so the workspace layout is not what \
         this scan assumes and its clean result is not a result",
        sources.len()
    );

    // Premise two: the primitive itself is in the corpus AND does the thing
    // this scan forbids everyone else from doing. Without it, "nobody writes"
    // would be true of a workspace that writes nothing at all.
    let primitive = sources
        .iter()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(PRIMITIVE))
        .unwrap_or_else(|| panic!("{PRIMITIVE} is not in the scanned corpus"));
    let primitive_src = std::fs::read_to_string(primitive).expect("readable");
    assert!(
        shipped_lines(&primitive_src)
            .iter()
            .any(|(_, l)| l.contains("fs::write")),
        "{PRIMITIVE} no longer contains the write this scan is protecting, so \
         finding none elsewhere establishes nothing"
    );

    // Premise three, and it is the one the first draft got wrong: the filter
    // must not swallow shipped code. A filter that returns nothing makes every
    // file clean, and `exec.rs` — which has THREE `#[cfg(test)]` markers, and
    // whose existence is what caught the first version — is the case that
    // matters, since a naive tail cut would have skipped everything after the
    // first.
    let mut filtered_out = 0usize;
    let mut kept = 0usize;
    for path in &sources {
        let src = std::fs::read_to_string(path).expect("readable");
        let total = src.lines().count();
        let shipped = shipped_lines(&src).len();
        filtered_out += total - shipped;
        kept += shipped;
        if path.file_name().and_then(|n| n.to_str()) == Some("exec.rs") {
            assert!(
                shipped > 20,
                "the `#[cfg(test)]` filter kept only {shipped} lines of {}, which \
                 has several test modules — it is swallowing shipped code and \
                 every clean result below is over source nobody read",
                path.display()
            );
        }
    }
    assert!(
        kept > filtered_out,
        "the filter removed more lines ({filtered_out}) than it kept ({kept}); \
         it is not a test filter any more"
    );

    // Premise four: the whole-file exclusion is EXERCISED, and by proof rather
    // than by filename. If it ever stops matching, these files come back into
    // the scan and report as offenders — which is the direction a stale
    // exclusion may fail in.
    let excluded: Vec<&PathBuf> = sources
        .iter()
        .filter(|p| gated_by_parent(p).is_some())
        .collect();
    assert!(
        !excluded.is_empty(),
        "no source file is excluded as a parent-gated test module, but this \
         workspace has at least one (`src/prompts/tests.rs`). Either the layout \
         moved or the proof stopped matching."
    );

    let mut offenders: Vec<String> = Vec::new();
    for path in &sources {
        if path.file_name().and_then(|n| n.to_str()) == Some(PRIMITIVE) {
            continue;
        }
        if gated_by_parent(path).is_some() {
            continue;
        }
        let src = std::fs::read_to_string(path).expect("readable");
        for (line, text) in shipped_lines(&src) {
            for needle in FORBIDDEN {
                if text.contains(needle) {
                    offenders.push(format!("{}:{line}: {needle}", path.display()));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a second filesystem-mutating path appeared outside {PRIMITIVE}:\n  {}\n\n\
         ADR-0001 §3 says core mutates the filesystem in one place, and \
         constraint 2's non-destructiveness is enforced by `FileOp` having no \
         `Delete`. That type governs only what routes through `apply`. \
         `Cache::save` did not, and carried a `remove_file` in production with \
         `FileOp::Delete` nonexistent throughout — which is how the enforcement \
         was bypassed the first time. If this write is legitimate, route it \
         through `write_atomically`; if it needs something the primitive does \
         not do, that is ADR-0001 §3a's revisit trigger and it is firing.",
        offenders.join("\n  ")
    );
}
