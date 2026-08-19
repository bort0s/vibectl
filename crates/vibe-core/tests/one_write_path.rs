//! **Nothing outside the primitive writes or removes file contents.**
//!
//! ADR-0001 §3 says core exposes no method that mutates the filesystem except
//! `apply`, and constraint 2's non-destructiveness is *"enforced by the absence
//! of `FileOp::Delete`, not by discipline."*
//!
//! **The type never enforced that, and `Cache::save` is the proof.** It wrote
//! and renamed on its own, and it carried a `std::fs::remove_file` — deletion,
//! in production, hand-written, with `FileOp::Delete` nonexistent throughout.
//! The type governs only what routes through `apply`; **nothing bounded what did
//! not.** The enforcement boundary is therefore not the type, it is whether a
//! call site routes through the primitive — a property of the source, which
//! until 2026-08-19 nothing checked.
//!
//! # The pattern is wider than the claim, on purpose
//!
//! *Amended 2026-08-19.* The first version enumerated five patterns —
//! `fs::write`, `fs::remove_file`, `fs::remove_dir_all`, `File::create`,
//! `.truncate(true)` — which made it an **inverted allowlist that did not say
//! so**: it proved the absence of those five, not the absence of mutation. Not
//! covered: `OpenOptions` with `.write(true)` and no `truncate`, `fs::rename`,
//! `fs::copy`, `fs::set_permissions`, symlink creation, or `use std::fs as f`.
//! Constraint 2 rests on this control, so the quantifier was the untested claim.
//!
//! **It matches every `fs::`, `File::` and `OpenOptions` now, and allows back by
//! name.** A false positive costs one allowlist line; a false negative cost
//! `Cache::save` surviving from P0. **Every allowlist has its size asserted**, so
//! it cannot grow quietly — which is the failure an allowlist exists to replace.
//!
//! And the name says what is asserted: **writes or removes file contents**, not
//! *mutates*. `create_dir_all` mutates and cannot destroy, and that line is where
//! the two words separate.
//!
//! # Reach, stated rather than assumed
//!
//! **Scanned:** every `.rs` under `crates/*/src/`, which is the shipped library
//! and binary code of every workspace member.
//!
//! **Not scanned, each with its reason and each bounded:**
//!
//! - `crates/*/tests/`, `#[cfg(test)]` items, and whole files gated
//!   `#[cfg(test)]` by an ancestor module. A fixture that plants a file is not
//!   the tool mutating a user's filesystem, and demanding it go through `apply`
//!   would mean building a plan to write a temp dir.
//! - `crates/*/examples/` — **the same reason, not a separate hole.** *Corrected
//!   2026-08-19: the first version called this "a real hole" and left it there,
//!   which is the awaiting-a-fix shape rather than a declared limit.* Examples
//!   are not installed and are not in the binary a user runs, and the one that
//!   mutates — `scan_bench.rs` — builds a synthetic corpus for a benchmark, which
//!   is a fixture. **Its size is asserted below**, so the region cannot become
//!   somewhere real code hides.
//! - `crates/*/benches/` — **there are none, and that is asserted**, so the day
//!   somebody adds one this turns red and asks to be extended rather than
//!   silently not covering it.
//! - Build scripts and `xtask` — none exist; a new `crates/*` sibling moves the
//!   corpus size and trips the first premise.

use std::path::{Path, PathBuf};

/// Where the one primitive lives. Everything else is checked against it.
const PRIMITIVE: &str = "plan.rs";

/// `std::fs` functions that **cannot** mutate anything.
///
/// Allowed by name because the property is in the function rather than in the
/// caller: no argument makes `read_dir` remove a file.
const READS: [&str; 7] = [
    "fs::read",
    "fs::read_to_string",
    "fs::read_dir",
    "fs::read_link",
    "fs::metadata",
    "fs::symlink_metadata",
    "fs::canonicalize",
];

/// Mutations that **cannot destroy**, allowed by name with the reason.
///
/// `create_dir_all` adds directories. It cannot truncate or remove anything, and
/// it fails rather than replacing when a file is in the way. Constraint 2 is
/// about destruction, and this is where *mutating* and *destroying* separate.
const HARMLESS_MUTATIONS: [&str; 1] = ["fs::create_dir_all"];

/// Files that write outside the primitive **by design**, with the reason.
///
/// One member, and it is not an exception to ADR-0001 §3 so much as a different
/// process: `monitor::writer` is what `vibe monitor hook` runs, spawned by
/// Claude Code, appending to a sink vibe manages (ADR-0011 §7a). It never
/// truncates, never removes, and never touches a file a user wrote — the whole
/// transport is one writer appending to its own file. Routing it through a
/// `WritePlan` would mean building a plan per hook invocation in a process whose
/// entire job is to append one line and exit.
const BY_DESIGN: [&str; 1] = ["writer.rs"];

fn workspace_crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits at <workspace>/crates/<name>")
        .join("crates")
}

fn rs_files_under(dir: &Path) -> Vec<PathBuf> {
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
    walk(dir, &mut out);
    out.sort();
    out
}

/// Every `crates/*/src/**/*.rs`.
fn shipped_sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for crate_dir in std::fs::read_dir(workspace_crates()).expect("crates/ is readable") {
        out.extend(rs_files_under(
            &crate_dir.expect("entry").path().join("src"),
        ));
    }
    out.sort();
    out
}

/// Whether a file is a test module gated `#[cfg(test)]` by **any** ancestor.
///
/// **This is the class, not the shape.** *Amended 2026-08-19.* The first version
/// looked only at the immediate parent, which happened to cover the one instance
/// it tripped over — `src/prompts/tests.rs`, gated by `src/prompts.rs` — and
/// would have missed a module gated two levels up. A filename establishes
/// nothing about gating, so the chain is walked toward the crate root and the
/// file is excluded if **any** link is gated.
fn gated_by_an_ancestor(path: &Path) -> bool {
    fn declared_gated(parent: &Path, stem: &str) -> Option<bool> {
        let src = std::fs::read_to_string(parent).ok()?;
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim();
            let declares = t == format!("mod {stem};")
                || t == format!("pub mod {stem};")
                || t == format!("pub(crate) mod {stem};");
            if declares {
                return Some(i > 0 && lines[i - 1].trim().starts_with("#[cfg(test)]"));
            }
        }
        None
    }

    let mut current = path.to_path_buf();
    // Bounded, because a cycle in the walk would hang the suite rather than
    // fail it. Eight is far past this workspace's depth.
    for _ in 0..8 {
        let Some(stem) = current
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_owned)
        else {
            return false;
        };
        let Some(dir) = current.parent().map(Path::to_path_buf) else {
            return false;
        };
        let candidates = [
            dir.with_extension("rs"),
            dir.join("mod.rs"),
            dir.join("lib.rs"),
            dir.join("main.rs"),
        ];
        let mut moved = false;
        for parent in candidates {
            if parent == current {
                continue;
            }
            match declared_gated(&parent, &stem) {
                Some(true) => return true,
                Some(false) => {
                    // Declared and not gated: keep walking up from the parent.
                    current = parent;
                    moved = true;
                    break;
                }
                None => {}
            }
        }
        if !moved {
            return false;
        }
    }
    false
}

/// Strip `#[cfg(test)]` items, so a fixture planting a file is not a finding.
///
/// **The first version cut from the first `#[cfg(test)]` to the end of the file,
/// and its own premise assertion caught it**: `exec.rs` has three, so a tail cut
/// would have skipped every shipped line below the first and reported a clean
/// scan over code it never looked at. That is the failure this whole file exists
/// against, one level in.
///
/// So it tracks braces instead. Braces inside string literals could still
/// confuse it, which is why the premises assert that shipped code **survives**
/// the filter rather than trusting that it does.
///
/// `use` lines are dropped too: an import is not a call site.
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
        if trimmed.starts_with("//") || trimmed.starts_with("use ") {
            continue;
        }
        out.push((i + 1, line));
    }
    out
}

/// Read a `fs::`/`File::` call name starting at `rest`.
fn call_name(rest: &str) -> String {
    rest.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '_')
        .collect()
}

/// Every filesystem call on a line that is not allowed back by name.
fn unallowed_calls(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (idx, _) in line.match_indices("fs::") {
        let name = call_name(&line[idx..]);
        if READS.contains(&name.as_str()) || HARMLESS_MUTATIONS.contains(&name.as_str()) {
            continue;
        }
        found.push(name);
    }
    for (idx, _) in line.match_indices("File::") {
        let name = call_name(&line[idx..]);
        if name == "File::open" {
            continue;
        }
        found.push(name);
    }
    if line.contains("OpenOptions") {
        found.push("OpenOptions".to_owned());
    }
    found
}

/// **The enforcement constraint 2 declares.**
#[test]
fn no_module_outside_the_primitive_writes_or_removes_file_contents() {
    let sources = shipped_sources();

    // Premise one: the scan found the corpus. A wrong root gives zero files, and
    // a gate asserting "no offenders" passes on zero perfectly.
    assert!(
        sources.len() > 20,
        "found only {} shipped source files, so the layout is not what this scan \
         assumes and its clean result is not a result",
        sources.len()
    );

    // Premise two: the primitive is in the corpus AND does what this forbids
    // everyone else. Without it, "nobody writes" is true of a workspace that
    // writes nothing at all.
    let primitive = sources
        .iter()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(PRIMITIVE))
        .unwrap_or_else(|| panic!("{PRIMITIVE} is not in the scanned corpus"));
    let primitive_src = std::fs::read_to_string(primitive).expect("readable");
    assert!(
        shipped_lines(&primitive_src)
            .iter()
            .any(|(_, l)| !unallowed_calls(l).is_empty()),
        "{PRIMITIVE} no longer performs the write this scan protects, so finding \
         none elsewhere establishes nothing"
    );

    // Premise three: the `#[cfg(test)]` filter must not swallow shipped code.
    // `exec.rs` is the case that matters — three markers, and a naive tail cut
    // would have skipped everything after the first.
    let (mut kept, mut cut) = (0usize, 0usize);
    for path in &sources {
        let src = std::fs::read_to_string(path).expect("readable");
        let shipped = shipped_lines(&src).len();
        kept += shipped;
        cut += src.lines().count() - shipped;
        if path.file_name().and_then(|n| n.to_str()) == Some("exec.rs") {
            assert!(
                shipped > 20,
                "the filter kept only {shipped} lines of exec.rs, which has \
                 several `#[cfg(test)]` items — it is swallowing shipped code"
            );
        }
    }
    assert!(
        kept > cut,
        "the filter removed more lines ({cut}) than it kept ({kept})"
    );

    // Premise four: the whole-file exclusion is exercised, and by proof rather
    // than by filename.
    let excluded: Vec<&PathBuf> = sources.iter().filter(|p| gated_by_an_ancestor(p)).collect();
    assert_eq!(
        excluded.len(),
        1,
        "expected exactly one ancestor-gated test module under `src/` \
         (`prompts/tests.rs`); found {excluded:?}. A new one is fine — say so \
         here — but it must not arrive unnoticed, because each is a file this \
         scan stops reading."
    );

    // Premise five: every allowlist's size is pinned, so none grows quietly.
    assert_eq!(READS.len(), 7, "the read allowlist changed size");
    assert_eq!(
        HARMLESS_MUTATIONS.len(),
        1,
        "a second `mutation that cannot destroy` was admitted. That is a \
         judgement about where constraint 2's boundary falls, and it belongs in \
         ADR-0001 rather than in a const"
    );
    assert_eq!(
        BY_DESIGN.len(),
        1,
        "a second module writes outside the primitive by design. That is \
         ADR-0001 §3a's revisit trigger firing: `core mutates the filesystem in \
         one place` has stopped being true"
    );

    // Premise six: `benches/` does not exist. The day it does, this turns red
    // and asks to be extended rather than quietly not covering it.
    for crate_dir in std::fs::read_dir(workspace_crates()).expect("readable") {
        let benches = crate_dir.expect("entry").path().join("benches");
        assert!(
            !benches.exists(),
            "{} exists and is outside this scan. Either add it to the corpus or \
             record why it is excluded — an unscanned directory nobody declared \
             is how `Cache::save` survived from P0",
            benches.display()
        );
    }

    // Premise seven: the excluded `examples/` region is BOUNDED.
    let examples: Vec<PathBuf> = std::fs::read_dir(workspace_crates())
        .expect("readable")
        .flatten()
        .flat_map(|c| rs_files_under(&c.path().join("examples")))
        .collect();
    assert_eq!(
        examples.len(),
        2,
        "the excluded `examples/` region changed size: {examples:?}. It is \
         excluded because an example is a fixture and is not installed, and that \
         reason has to be re-checked when it grows"
    );

    let mut offenders: Vec<String> = Vec::new();
    for path in &sources {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == PRIMITIVE || BY_DESIGN.contains(&name) || gated_by_an_ancestor(path) {
            continue;
        }
        let src = std::fs::read_to_string(path).expect("readable");
        for (line, text) in shipped_lines(&src) {
            for call in unallowed_calls(text) {
                offenders.push(format!("{}:{line}: {call}", path.display()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a filesystem call outside {PRIMITIVE} that is neither a read nor \
         allowed by name:\n  {}\n\n\
         ADR-0001 §3 says core mutates the filesystem in one place, and \
         constraint 2's non-destructiveness is enforced by `FileOp` having no \
         `Delete` — a type that governs only what routes through `apply`. \
         `Cache::save` did not, and carried a `remove_file` in production with \
         `FileOp::Delete` nonexistent throughout. If this call is legitimate, \
         route it through `write_atomically`; if it cannot be, that is \
         ADR-0001 §3a's revisit trigger and it is firing.",
        offenders.join("\n  ")
    );
}
