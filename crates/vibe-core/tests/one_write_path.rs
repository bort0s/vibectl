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
//! - `crates/*/examples/` — excluded for a **weaker reason than the tests are,
//!   and that asymmetry is the point.** *Amended 2026-08-19.* A `#[cfg(test)]`
//!   item is excluded **structurally**: it is not compiled into the library a
//!   user links, so it cannot be the tool mutating anything. An example is
//!   excluded on a **judgement about its content**, and nothing structural stops
//!   an example from doing real work tomorrow. So each one is listed in
//!   [`EXAMPLES`] **with the reason it is excluded**, and the directory must
//!   match that list exactly — a new example turns this red until somebody makes
//!   the call about it. Only one of them mutates outside the primitive
//!   (`scan_bench.rs`, a benchmark fixture builder); the other two invoke
//!   `write_atomically`, which is the shipped path rather than a second one.
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
///
/// # THE HOLE THIS OPENS, AND WHAT CLOSES IT
///
/// *Added 2026-08-19.* Allowlisting by **module** disables every pattern in it —
/// including `.truncate(true)`, in the one production site where truncation
/// would destroy records rather than a user's file. The control that would have
/// noticed is switched off exactly where it matters most.
///
/// Two repairs were available: make the allowlist per-pattern, or give the
/// module its own control. **The second, because it asserts a property of the
/// write path rather than a property of the matcher**, and it survives the
/// allowlist being restructured. See
/// [`the_hooks_append_never_truncates`] and
/// [`a_second_append_does_not_replace_the_first`] — source and effect, in that
/// order of authority.
const BY_DESIGN: [&str; 1] = ["writer.rs"];

/// Every file under `crates/*/examples/`, **with the judgement that excludes
/// it**, because a count is not a judgement.
///
/// *Changed 2026-08-19 from a bare count of 2, which the rebase promptly turned
/// red — correctly: a third example arrived and the assertion said "this number
/// moved" rather than "make the call about this file". The number was the thing
/// being asserted, and the number was never the point.*
///
/// Examples are excluded on a **judgement about content**, not structurally the
/// way `#[cfg(test)]` is (see the module docs). So the judgement is written down
/// per file and the directory has to match this list exactly — a new example
/// fails until somebody adds it here, which is the judgement being made rather
/// than inherited.
///
/// # DECLARED LIMIT: the judgement is per file and it does not expire
///
/// *Added 2026-08-19.* This catches a **new** example. It does not catch an
/// existing one changing: `scan_bench.rs` builds fixtures today, and if it grew
/// a write to somewhere real tomorrow neither this list nor its length would
/// move. So the exclusion is sound at the moment each line was written and
/// ages from then on, which is exactly the asymmetry with `#[cfg(test)]` —
/// **that one cannot rot, because it is the compiler's exclusion and not
/// somebody's reading.**
///
/// **What would close it** is scanning examples with a narrower rule of their
/// own, rather than excluding them. Not built: it costs a second policy for
/// four files, and the failure it guards — an example that writes somewhere
/// real — is not reachable by a user, since `cargo install` does not ship them.
/// Recorded as a limit with its reason rather than as a hole awaiting a fix.
const EXAMPLES: [(&str, &str); 3] = [
    (
        "atomic_replace_once.rs",
        "one call to `write_atomically` — it EXERCISES the primitive rather than \
         reimplementing a write, which is what an example touching the \
         filesystem has to do to stay excluded",
    ),
    (
        "emit_install_settings.rs",
        "reads a settings file, installs into it and writes back through \
         `write_atomically` — the shipped path, invoked, not a second one",
    ),
    (
        "scan_bench.rs",
        "builds a synthetic corpus for a benchmark with direct `fs::write`, \
         `create_dir_all` and `rename`. A FIXTURE BUILDER, the same class as a \
         test that plants files — and the only member here that mutates \
         outside the primitive, which is why the exclusion is a judgement",
    ),
];

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
    let mut found: Vec<String> = std::fs::read_dir(workspace_crates())
        .expect("readable")
        .flatten()
        .flat_map(|c| rs_files_under(&c.path().join("examples")))
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned))
        .collect();
    found.sort();
    let mut judged: Vec<String> = EXAMPLES.iter().map(|(n, _)| (*n).to_owned()).collect();
    judged.sort();
    assert_eq!(
        found, judged,
        "the set of files under `examples/` is not the set that has been judged. \
         They are excluded on a JUDGEMENT ABOUT CONTENT — not structurally the \
         way `#[cfg(test)]` is — so a new one has to have that judgement made \
         about it and written into `EXAMPLES`, rather than inheriting an \
         exclusion somebody else earned."
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

// ---------------------------------------------------------------------------
// The hole the by-design allowlist opens, closed at the module it points at
// ---------------------------------------------------------------------------

/// **The hook's append must never truncate**, and this is the source half.
///
/// `writer.rs` is in [`BY_DESIGN`], which disables every pattern in it — so
/// `.truncate(true)` there would not turn the scan above red. That is the one
/// production site where truncation destroys **records**, and the only one where
/// the control that would notice is switched off. The exclusion is what creates
/// the hole, so the repair lives beside the exclusion.
///
/// `the_write_path_has_no_buffered_writer` reads that file for `BufWriter`,
/// which is a different property: nothing there asserted the **open mode**.
///
/// **Premise-asserting**, because this is a source read and its silence is
/// worthless if the file moved: the `OpenOptions` call it reasons about must be
/// found, exactly once, on a line that is not a comment.
#[test]
fn the_hooks_append_never_truncates() {
    let writer_rs = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("monitor")
        .join("writer.rs");
    let src = std::fs::read_to_string(&writer_rs)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", writer_rs.display()));
    let lines = shipped_lines(&src);

    let opens: Vec<&(usize, &str)> = lines
        .iter()
        .filter(|(_, l)| l.contains("OpenOptions::new()"))
        .collect();
    assert_eq!(
        opens.len(),
        1,
        "expected exactly one `OpenOptions::new()` in {}, found {}. The open \
         this control reasons about moved, so finding no truncation in the file \
         establishes nothing about the append.",
        writer_rs.display(),
        opens.len()
    );

    let open_line = opens[0].1;
    assert!(
        open_line.contains(".append(true)"),
        "the sink is not opened in append mode: {open_line:?}. Every record \
         already in the file is at risk, and ADR-0011 §7a's whole transport is \
         one writer APPENDING to its own file."
    );

    for (line, text) in &lines {
        assert!(
            !text.contains(".truncate(true)"),
            "{}:{line} truncates. This module is allowlisted in the scan above, \
             so nothing else would have caught it — and truncation here destroys \
             a session's records rather than a user's file.",
            writer_rs.display()
        );
    }
}

/// **The effect half, which is the one that survives a rewrite.**
///
/// The source check catches `.truncate(true)` by name. This catches truncation
/// however it arrives — a different open, a different API, a `set_len` — by
/// asserting the only thing that actually matters: **a second append does not
/// cost the first record.**
#[test]
fn a_second_append_does_not_replace_the_first() {
    use std::sync::Arc;
    use vibe_core::monitor::{SystemStamps, WriteOutcome, Writer, WriterIdentity};

    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");
    let writer = Writer::new(
        &sink,
        WriterIdentity::parse("ident").expect("valid"),
        Arc::new(SystemStamps),
    );

    let payload =
        |event: &str| format!(r#"{{"session_id":"s","hook_event_name":"{event}","cwd":"/tmp/p"}}"#);
    let first = writer.append(&payload("SessionStart"));
    let second = writer.append(&payload("SessionEnd"));

    let path = match (&first, &second) {
        (WriteOutcome::Written { path, .. }, WriteOutcome::Written { .. }) => path.clone(),
        other => panic!("both appends must land: {other:?}"),
    };

    let text = std::fs::read_to_string(&path).expect("read");
    assert!(
        text.contains("SessionStart"),
        "the first record is gone after a second append — the sink is being \
         truncated, and every session's history goes with it. File held:\n{text}"
    );
    assert!(text.contains("SessionEnd"));
    assert_eq!(
        text.lines().count(),
        2,
        "expected two records, got:\n{text}"
    );
}
