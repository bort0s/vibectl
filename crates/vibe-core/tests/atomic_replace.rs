//! **A replacement is never observed partial**, and the instrument is proved
//! able to see one.
//!
//! Three rounds of ADR-0011 went into whether a killed hook could tear a record
//! in vibe's own sink. None of it transfers to this path: here vibe rewrites
//! files whose readers have no tolerance at all — `.claude/settings.json` is
//! read by a strict JSON loader — and one of them is a file vibe **does not
//! own**. Hard constraint 2 is *never destructive*; the missing
//! `FileOp::Delete` is how it is enforced, not what it says.
//!
//! `std::fs::write` is `File::create` plus `write_all`, and `File::create`
//! **truncates before any byte is written**. The zero-byte window is therefore
//! not a race that might not happen — it is a state the sequence passes through
//! every time. That is a stronger hazard than the one three rounds were spent
//! failing to reproduce, and it was sitting in `apply` the whole while.
//!
//! # The pairing is the whole design
//!
//! A reader spins on the target while a writer replaces it many times. With the
//! atomic route it must observe **only whole contents**. That result means
//! nothing on its own — a reader too slow to catch anything reports it too — so
//! the identical reader runs against `std::fs::write` and **must** catch a
//! non-whole state. One instrument, two write modes, and the negative half is
//! what licenses the positive one.
//!
//! Runs in the ordinary test job, so *"rename replaces without a window"* is
//! carried on all three platforms rather than read off documentation. It is
//! exactly the class of cross-platform claim ADR-0002 §7 records dying on
//! contact with measurement.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use vibe_core::write_atomically;

/// Distinct states a reader can observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Seen {
    Missing,
    Empty,
    Partial,
    WholeOld,
    WholeNew,
}

const OLD: &str = "OLD-CONTENTS-OLD-CONTENTS-OLD-CONTENTS-OLD-CONTENTS-OLD\n";
const NEW: &str = "NEW-CONTENTS-NEW-CONTENTS-NEW-CONTENTS-NEW-CONTENTS-NEW\n";

fn classify(bytes: Option<Vec<u8>>) -> Seen {
    match bytes {
        None => Seen::Missing,
        Some(b) if b.is_empty() => Seen::Empty,
        Some(b) if b == OLD.as_bytes() => Seen::WholeOld,
        Some(b) if b == NEW.as_bytes() => Seen::WholeNew,
        Some(_) => Seen::Partial,
    }
}

/// Spin-read `target` until told to stop, collecting every distinct state.
fn observe(target: &Path, stop: &Arc<AtomicBool>) -> Vec<Seen> {
    let mut seen: Vec<Seen> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        let state = classify(std::fs::read(target).ok());
        if !seen.contains(&state) {
            seen.push(state);
        }
    }
    seen.sort_unstable();
    seen
}

/// Run `replace` many times under a spinning reader and report what it saw.
fn states_under(replace: fn(&Path, &str), rounds: usize) -> Vec<Seen> {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("target.json");
    std::fs::write(&target, OLD).expect("seed");

    let stop = Arc::new(AtomicBool::new(false));
    let reader_stop = Arc::clone(&stop);
    let reader_target = target.clone();
    let reader = std::thread::spawn(move || observe(&reader_target, &reader_stop));

    for i in 0..rounds {
        replace(&target, if i % 2 == 0 { NEW } else { OLD });
    }

    stop.store(true, Ordering::Relaxed);
    reader.join().expect("reader thread")
}

fn atomic(path: &Path, contents: &str) {
    write_atomically(path, contents).expect("atomic write");
}

fn truncating(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("truncating write");
}

/// **The negative half, and it runs first because the positive half means
/// nothing without it.**
///
/// The identical reader against `std::fs::write` must catch the target in a
/// state that is not a whole document. If it cannot, it is too slow to catch
/// anything and every clean result below is the reader's, not the writer's.
#[test]
fn the_truncating_write_is_caught_mid_replacement() {
    let seen = states_under(truncating, 400);
    let bad: Vec<Seen> = seen
        .iter()
        .copied()
        .filter(|s| matches!(s, Seen::Missing | Seen::Empty | Seen::Partial))
        .collect();
    assert!(
        !bad.is_empty(),
        "the reader never caught `std::fs::write` between its truncate and its \
         write, so it is not fast enough to establish anything about the atomic \
         route either. States seen: {seen:?}"
    );
    println!("truncating write, states observed: {seen:?}");
}

/// **The atomic route is never observed partial.**
#[test]
fn a_replace_is_never_observed_partial() {
    let seen = states_under(atomic, 400);
    let bad: Vec<Seen> = seen
        .iter()
        .copied()
        .filter(|s| matches!(s, Seen::Missing | Seen::Empty | Seen::Partial))
        .collect();
    assert!(
        bad.is_empty(),
        "a reader saw {bad:?} while `write_atomically` replaced the target on \
         {}/{}. Rename-over-existing is not replacing without a window on this \
         platform, and a settings.json can therefore be observed destroyed. \
         All states seen: {seen:?}",
        std::env::consts::OS,
        std::env::consts::ARCH,
    );

    // And it really did replace, or "no bad states" is satisfied by a writer
    // that never wrote.
    assert!(
        seen.contains(&Seen::WholeNew) && seen.contains(&Seen::WholeOld),
        "the reader never saw both contents, so the replacements did not happen \
         under it. States seen: {seen:?}"
    );
    println!("atomic replace, states observed: {seen:?}");
}

/// The temp file lands **beside the target**, because a rename across volumes
/// is a copy plus a delete — which puts the window back and adds a delete to a
/// tool that has none.
///
/// Asserted by observing the directory during the write rather than by reading
/// the implementation, so a later change that moves the temp elsewhere is
/// caught by its effect.
#[test]
fn the_temporary_file_is_written_beside_the_target() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("nested").join("target.json");
    std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
    std::fs::write(&target, OLD).expect("seed");

    let stop = Arc::new(AtomicBool::new(false));
    let reader_stop = Arc::clone(&stop);
    let watched = target.parent().expect("parent").to_path_buf();
    let reader = std::thread::spawn(move || {
        let mut names: Vec<String> = Vec::new();
        while !reader_stop.load(Ordering::Relaxed) {
            if let Ok(entries) = std::fs::read_dir(&watched) {
                for e in entries.flatten() {
                    let n = e.file_name().to_string_lossy().into_owned();
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
            }
        }
        names
    });

    for _ in 0..400 {
        write_atomically(&target, NEW).expect("write");
    }
    stop.store(true, Ordering::Relaxed);
    let names = reader.join().expect("reader");

    assert!(
        names.iter().any(|n| n.ends_with(".tmp")),
        "no temporary file was ever seen in the target's own directory, so \
         either the write is not going through a temp file or the temp file is \
         on another volume — where a rename is a copy plus a delete. Saw: \
         {names:?}"
    );
    // Nothing is left behind once the writes are done.
    let leftovers: Vec<_> = std::fs::read_dir(target.parent().expect("parent"))
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );
}
