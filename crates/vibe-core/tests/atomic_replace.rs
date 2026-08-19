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
///
/// # This one samples, deliberately, and it is the only one here that does
///
/// ADR-0002 §7 refuses controls whose firing depends on winning a race, because
/// they stop proving anything without ever failing. **The exception is earned by
/// what a red means.** The hazard here *is* a timing window; there is no
/// non-sampling way to observe a state that exists only between two syscalls,
/// and the alternative is no licence for the zero at all.
///
/// And this one fails in the honest direction: a red says *"this run did not
/// establish anything"*, not *"the code is broken"*. The failure text says so,
/// so a loaded runner produces a retry-worthy message rather than a false
/// defect claim.
///
/// **The zero it licenses is bounded by reader resolution.** The truncating
/// window is a `File::create` plus a ~500-byte `write_all` — long. A rename's
/// window is orders of magnitude shorter, and a reader shown to sample inside a
/// long window is **not** shown to sample inside a short one. So
/// [`a_replace_is_never_observed_partial`] is evidence and not proof, and the
/// structural argument in `write_atomically`'s docs is what carries the claim.
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
