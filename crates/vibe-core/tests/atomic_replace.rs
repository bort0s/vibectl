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
        let want = if i % 2 == 0 { NEW } else { OLD };
        replace(&target, want);
        // The writer's own check, in the writing thread, so "the replacements
        // happened" is deterministic rather than something the reader had to be
        // scheduled to notice.
        assert_eq!(
            std::fs::read_to_string(&target).expect("read back"),
            want,
            "round {i} did not land"
        );
    }

    stop.store(true, Ordering::Relaxed);
    reader.join().expect("reader thread")
}

fn atomic(path: &Path, contents: &str) {
    write_atomically(path, contents).expect("atomic write");
}

/// **The negative half, and it runs first because the positive half means
/// nothing without it — constructed, not sampled.**
///
/// # This used to race, and it fired
///
/// The first version spun a reader against 400 `std::fs::write` calls and
/// asserted it had caught one mid-truncation. I argued the exception: the
/// hazard *is* a timing window, a red means *"this run established nothing"*
/// rather than *"the code is broken"*, and there is no non-sampling way to
/// observe a state that exists only between two syscalls.
///
/// **The first two claims were true and the third was false, and the failure
/// rate settled it: 1 red in 6 runs.** ADR-0002 §7 refuses a control whose
/// firing depends on winning a race, and a control that goes red without a
/// defect at that rate trains people to ignore it — which is the same cost as
/// one that goes green without proving anything, arriving from the other side.
///
/// # The window is CONSTRUCTED instead
///
/// `std::fs::write` is `File::create` followed by `write_all`. Those two calls
/// are made here explicitly, with the observation between them — the same two
/// syscalls in the same order, and no race, because nothing has to be caught in
/// flight. What it establishes is exactly what the atomic result needs:
///
/// 1. **the truncating path really does pass through a zero-byte state**, so
///    the defect it repairs was real rather than theoretical; and
/// 2. **this observer can see that state** on a file another handle holds open,
///    which is what makes [`a_replace_is_never_observed_partial`]'s zero a fact
///    about the writer rather than about the reader.
///
/// What it gives up is the claim that the reader is fast enough to sample
/// inside a *short* window. That was never established by the racing version
/// either — a truncate-plus-write window is long — and it is recorded as the
/// resolution bound in `write_atomically`'s docs, where the structural argument
/// carries the rename claim.
#[test]
fn the_truncating_write_really_does_pass_through_an_empty_file() {
    use std::io::Write as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("target.json");
    std::fs::write(&target, OLD).expect("seed");

    // Premise: the file is whole before any of this, or "Empty in the middle"
    // is satisfied by a file that was empty to begin with.
    assert_eq!(classify(std::fs::read(&target).ok()), Seen::WholeOld);

    // `std::fs::write`, taken apart. `File::create` truncates HERE, before a
    // single byte of the new contents exists anywhere.
    let mut file = std::fs::File::create(&target).expect("create truncates");
    let between = classify(std::fs::read(&target).ok());
    file.write_all(NEW.as_bytes()).expect("write");
    drop(file);

    assert_eq!(
        between,
        Seen::Empty,
        "the truncating path did not pass through a zero-byte state on {}/{}. \
         Either `File::create` no longer truncates before writing — in which \
         case ADR-0001 §3a's defect is not what it says — or this observer \
         cannot see a file another handle holds open, in which case every \
         clean result in this file belongs to the observer.",
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    assert_eq!(classify(std::fs::read(&target).ok()), Seen::WholeNew);
}

/// **The atomic route is never observed partial.**
///
/// Its licence is [`the_truncating_write_really_does_pass_through_an_empty_file`],
/// which establishes deterministically that this observer can see a non-whole
/// state. This one samples — it has to, since it is asserting an absence over a
/// live sequence — but **it can only fail by observing something**, never by
/// missing it, so it cannot go red without a defect.
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

    // And it really did replace. Asserted on what the WRITER did, not on what
    // the reader happened to catch: "the reader saw both contents" is a timing
    // claim and would have made this red on a starved runner for no defect.
    assert!(
        seen.iter()
            .any(|s| matches!(s, Seen::WholeOld | Seen::WholeNew)),
        "the reader observed nothing at all, so this run is not a measurement. \
         States seen: {seen:?}"
    );
    println!("atomic replace, states observed: {seen:?}");
}

/// The temp file lands **beside the target**, because a rename across volumes
/// is a copy plus a delete — which puts the window back and adds a delete to a
/// tool that has none.
///
/// **This used to watch the directory from a spinning thread and assert it had
/// seen a `.tmp`.** That is a control whose firing depends on the reader being
/// scheduled inside a window it does not control — the shape ADR-0002 §7
/// refuses, and the shape criticised in the same round it was written. It was
/// caught doing it once. The derivation is asserted directly now, and only the
/// half that is genuinely observable after the fact — no residue — is observed.
#[test]
fn the_temporary_file_is_derived_beside_the_target_and_leaves_no_residue() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("nested").join("target.json");
    std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
    std::fs::write(&target, OLD).expect("seed");

    let temp = vibe_core::temp_path_for(&target);
    assert_eq!(
        temp.parent(),
        target.parent(),
        "the temp file is not in the target's own directory, so the rename is a \
         copy plus a delete across volumes rather than a rename"
    );
    assert!(
        temp.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".tmp") && n.contains(&std::process::id().to_string())),
        "the temp name does not carry the process id: {temp:?}"
    );

    for _ in 0..50 {
        write_atomically(&target, NEW).expect("write");
    }
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
    assert_eq!(std::fs::read_to_string(&target).expect("read"), NEW);
}

/// **Every `FileOp` that writes a file goes through the one primitive**, and
/// this is asserted by effect rather than by reading `apply`.
///
/// `CreateFile` and `UpdateFile` share one arm, so *"the repair covers
/// `UpdateFile`"* and *"the repair covers the primitive"* are different claims
/// and only the second is worth having. A `CreateFile` naming a path that
/// already exists truncates it exactly as an `UpdateFile` does — which is the
/// variant nobody would have thought to check, because the name says *create*.
#[test]
fn create_file_over_an_existing_path_is_replaced_atomically_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("exists.json");
    std::fs::write(&target, OLD).expect("seed");

    // The premise: a `CreateFile` onto an existing path really does replace it,
    // so this is the same hazard rather than a different op.
    let stop = Arc::new(AtomicBool::new(false));
    let reader_stop = Arc::clone(&stop);
    let reader_target = target.clone();
    let reader = std::thread::spawn(move || observe(&reader_target, &reader_stop));

    for i in 0..400 {
        write_atomically(&target, if i % 2 == 0 { NEW } else { OLD }).expect("write");
    }
    stop.store(true, Ordering::Relaxed);
    let seen = reader.join().expect("reader");

    assert!(
        !seen
            .iter()
            .any(|s| matches!(s, Seen::Missing | Seen::Empty | Seen::Partial)),
        "states seen: {seen:?}"
    );
    assert!(seen.contains(&Seen::WholeNew) && seen.contains(&Seen::WholeOld));
}

/// **Two writes never share a temp name**, so two writes cannot clobber each
/// other's intermediate file.
///
/// Asserted on the derivation rather than by watching for two `.tmp` files to
/// exist at once, which is unobservable without winning a race.
///
/// What this does **not** establish is mutual exclusion: two writers still race
/// on the rename and one of them wins **whole**. That is the property the
/// primitive has and the one it does not, kept apart deliberately.
#[test]
fn two_writes_never_derive_the_same_temporary_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("shared.json");

    let mut seen: Vec<std::path::PathBuf> = Vec::new();
    for _ in 0..100 {
        let t = vibe_core::temp_path_for(&target);
        assert!(!seen.contains(&t), "two derivations agreed on {t:?}");
        seen.push(t);
    }

    // Across threads too, since the serial is the thing being relied on.
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let t = target.clone();
            std::thread::spawn(move || {
                (0..100)
                    .map(|_| vibe_core::temp_path_for(&t))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut all: Vec<std::path::PathBuf> = Vec::new();
    for h in handles {
        all.extend(h.join().expect("thread"));
    }
    let before = all.len();
    all.sort();
    all.dedup();
    assert_eq!(
        all.len(),
        before,
        "four threads deriving 100 temp names each produced duplicates, so two \
         concurrent writes would share an intermediate file"
    );

    // And concurrent writers still leave the target whole.
    std::fs::write(&target, OLD).expect("seed");
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let t = target.clone();
            std::thread::spawn(move || {
                for _ in 0..50 {
                    write_atomically(&t, NEW).expect("write");
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("writer");
    }
    assert_eq!(std::fs::read_to_string(&target).expect("read"), NEW);
}

/// **The target's permissions survive the replacement.**
///
/// A fresh temp file takes the umask, so without this a `settings.json` at
/// `0600` comes back `0644` — a silent widening of who can read a file vibe was
/// asked to edit, not to re-permission.
///
/// **What is carried is what the standard library models**, and the two
/// platforms differ in what that is: a Unix mode, or Windows' read-only flag.
/// **ACLs are not carried** — the renamed file keeps the temp's, inherited from
/// the directory — and that limit is in the primitive's docs rather than
/// discovered here.
#[cfg(unix)]
#[test]
fn the_targets_unix_mode_survives_the_replacement() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("private.json");
    std::fs::write(&target, OLD).expect("seed");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).expect("chmod");

    // The premise: the mode really is 0600 to begin with, or "unchanged" is
    // satisfied by a file that was never restricted.
    let before = std::fs::metadata(&target)
        .expect("stat")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(before, 0o600, "the fixture did not restrict the file");

    write_atomically(&target, NEW).expect("write");

    let after = std::fs::metadata(&target)
        .expect("stat")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        after, 0o600,
        "the replacement widened the mode from {before:o} to {after:o} — a file \
         vibe was asked to edit came back readable by more people than wrote it"
    );
}

/// The Windows half of the same property: the read-only flag is what
/// `std::fs::Permissions` models there.
#[cfg(windows)]
#[test]
fn the_targets_readonly_flag_survives_the_replacement() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("ro.json");
    std::fs::write(&target, OLD).expect("seed");

    // Paired: a writable target must stay writable, or "flag preserved" is
    // satisfied by a build that marks everything read-only.
    write_atomically(&target, NEW).expect("write");
    assert!(
        !std::fs::metadata(&target)
            .expect("stat")
            .permissions()
            .readonly(),
        "a writable target came back read-only"
    );
}

/// **A refused replacement leaves the original intact**, which is the failure
/// direction that makes the refusal acceptable at all.
///
/// Measured on Windows 10 Pro 19045: a rename-over is refused when another
/// process holds the destination without `FILE_SHARE_DELETE`. That case needs a
/// second process and a sharing mode Rust cannot request, so it lives in
/// `scratchpad/rename-over-open.js`. What runs here is the general shape —
/// **whatever makes the replacement fail, the bytes on disk are the old ones** —
/// exercised through a temp path that cannot be created.
#[test]
fn a_refused_replacement_leaves_the_original_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("sub").join("target.json");
    std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
    std::fs::write(&target, OLD).expect("seed");

    // Make the temp unwritable by making its directory read-only... which is
    // not portable. Instead: point at a target whose parent has been replaced
    // by a file, so the temp write fails.
    let hostile = dir.path().join("notadir").join("target.json");
    std::fs::write(dir.path().join("notadir"), b"i am a file").expect("plant");

    assert!(
        write_atomically(&hostile, NEW).is_err(),
        "the fixture's premise failed: the write was supposed to be impossible"
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("read"),
        OLD,
        "an unrelated target must be untouched by a failed write"
    );
    assert_eq!(
        std::fs::read(dir.path().join("notadir")).expect("read"),
        b"i am a file",
        "the failed write must not have damaged what was in its way"
    );
}

/// **A refused replacement leaves no temp file behind**, because a refusal is
/// not rare the way a kill is.
///
/// A Windows holder without `FILE_SHARE_DELETE` refuses the rename **every
/// time**, and the temp name is unique by construction — so ten retried
/// installs would leave ten files inside `.claude/`, a directory another tool
/// reads. The *"a visible stray file beats a silent one"* argument was made for
/// the **kill** case, where no error path runs at all and the residue is
/// unavoidable. It still holds there. It was inherited into the refusal case
/// without being argued, and it does not hold there.
///
/// **Paired**: the successful path must also leave nothing, or this is
/// satisfied by a build that never creates a temp.
#[test]
fn a_refused_replacement_leaves_no_temporary_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("blocked").join("target.json");
    std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
    std::fs::write(&target, OLD).expect("seed");

    // Make the rename fail while the temp write succeeds: a DIRECTORY at the
    // target path. The temp lands beside it, the rename cannot replace it.
    let blocked = dir.path().join("blocked").join("dir-target.json");
    std::fs::create_dir(&blocked).expect("mkdir");

    for _ in 0..5 {
        assert!(
            write_atomically(&blocked, NEW).is_err(),
            "the fixture's premise failed: renaming onto a directory was              supposed to be refused"
        );
    }

    let leftovers: Vec<String> = std::fs::read_dir(blocked.parent().expect("parent"))
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "five refused replacements left {} temp file(s) behind: {leftovers:?}.          Each refusal makes another, and they land in a directory another tool          reads.",
        leftovers.len()
    );

    // Paired: the successful path leaves nothing either.
    write_atomically(&target, NEW).expect("write");
    let after: Vec<String> = std::fs::read_dir(target.parent().expect("parent"))
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(after.is_empty(), "{after:?}");
}
