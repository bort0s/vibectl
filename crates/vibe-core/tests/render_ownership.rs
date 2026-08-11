//! The ownership contract for rendered files (ADR-0007).
//!
//! `render` writes into a directory the user owns, and one of its targets —
//! `README.md` — is routinely the most valuable prose file in a repository. So
//! the interesting assertions here are all about **refusing**, and the one that
//! matters most is that `--force` does not override `Foreign`.
//!
//! Every negative control in this file is **paired**, per ADR-0002 §7: each
//! asserts the guard fires under the hazard *and* stays quiet without it. A
//! one-sided control passes just as well against a build that refuses
//! unconditionally, which is a different bug with the same symptom — a `render`
//! that never renders.

use vibe_core::render::{RenderState, marker};

const BODY: &str = "# macroring\n\nMobile-first PWA for nutrition tracking.\n";

/// A file as `vibe render` would have written it.
fn generated(body: &str) -> String {
    marker::wrap(body)
}

#[test]
fn a_file_we_wrote_and_nobody_touched_is_generated() {
    let file = generated(BODY);
    assert_eq!(marker::classify(&file), RenderState::Generated);
    assert!(RenderState::Generated.may_overwrite(false));
}

#[test]
fn a_file_with_no_marker_is_foreign() {
    assert_eq!(
        marker::classify("# My hand-written README\n"),
        RenderState::Foreign
    );
    assert_eq!(marker::classify(""), RenderState::Foreign);
}

/// **The assertion `README.md` as a target rests on.**
///
/// The dangerous case is not "the user edited our README" — it is "the user has
/// a README and we never wrote it". `--force` means "discard my edits to *your*
/// file"; it is not a way to claim a file `vibe` never wrote.
#[test]
fn force_does_not_override_foreign_and_does_override_modified() {
    // Paired: force is the difference between these two, and the state is the
    // difference between the two rows.
    assert!(!RenderState::Foreign.may_overwrite(false));
    assert!(
        !RenderState::Foreign.may_overwrite(true),
        "--force must never adopt a file vibe did not write; that is what makes \
         README.md safe to have as a target at all"
    );

    assert!(!RenderState::Modified.may_overwrite(false));
    assert!(
        RenderState::Modified.may_overwrite(true),
        "--force is the user saying yes, discard my edits to your file"
    );
}

#[test]
fn an_edited_generated_file_is_modified_not_generated() {
    let mut file = generated(BODY);
    file.push_str("\nA paragraph I wrote by hand.\n");
    assert_eq!(marker::classify(&file), RenderState::Modified);
    assert!(!RenderState::Modified.may_overwrite(false));
}

/// The reason the hash is normalised before it is taken.
///
/// `git` converts line endings on checkout under `core.autocrlf`, the default
/// on Windows. A hash over raw bytes would break on the next clone and report
/// **every rendered file in the repository** as modified — offering to
/// overwrite work nobody touched, on a machine that did nothing wrong.
#[test]
fn a_crlf_checkout_of_our_own_file_is_still_generated() {
    let unix = generated(BODY);
    let windows = unix.replace('\n', "\r\n");
    assert_ne!(unix, windows, "the fixture must actually differ in bytes");

    assert_eq!(marker::classify(&unix), RenderState::Generated);
    assert_eq!(
        marker::classify(&windows),
        RenderState::Generated,
        "a CRLF checkout is the same file; reporting it as Modified would make \
         every rendered file in a Windows clone look edited"
    );

    // Paired: normalisation must not be so eager that it hides a real edit.
    let edited = windows.replace("Mobile-first", "Desktop-first");
    assert_eq!(
        marker::classify(&edited),
        RenderState::Modified,
        "normalising line endings must not normalise away content"
    );
}

/// Trailing-whitespace-only differences are the other thing editors do on save.
#[test]
fn a_trailing_newline_difference_is_not_an_edit_but_a_content_change_is() {
    let base = generated(BODY);
    assert_eq!(
        marker::classify(base.trim_end()),
        RenderState::Generated,
        "an editor stripping the final newline is not an edit"
    );
    assert_eq!(
        marker::classify(&format!("{base}\n\n\n")),
        RenderState::Generated
    );
    // Paired.
    assert_eq!(
        marker::classify(&format!("{base}extra content\n")),
        RenderState::Modified
    );
}

/// ADR-0006 §3's discipline, applied to a second format: an unrecognised
/// algorithm is "cannot tell", never "differs".
#[test]
fn an_unknown_hash_algorithm_is_unverifiable_not_modified() {
    let file = generated(BODY);
    let forged = file.replacen("hash=b3:", "hash=sha256:", 1);
    assert_ne!(
        forged, file,
        "the fixture must actually change the algorithm"
    );

    assert_eq!(
        marker::classify(&forged),
        RenderState::Unverifiable,
        "reporting this as Modified would flag every rendered file as edited the \
         day the algorithm changes"
    );
    assert!(!RenderState::Unverifiable.may_overwrite(false));
    assert!(RenderState::Unverifiable.may_overwrite(true));
}

/// A marker from a future marker-format version is readable enough to prove
/// ownership, but its integrity claim is not one this build can evaluate.
#[test]
fn a_newer_marker_version_is_unverifiable_rather_than_foreign() {
    let file = generated(BODY).replacen("vibe:generated v1", "vibe:generated v9", 1);
    assert_eq!(
        marker::classify(&file),
        RenderState::Unverifiable,
        "it is still our marker, so it is not Foreign; but this build cannot \
         vouch for the rest of the line"
    );
}

#[test]
fn the_marker_is_the_first_line_and_survives_a_round_trip() {
    let file = generated(BODY);
    let first = file.lines().next().expect("a first line");
    assert!(first.contains("vibe:generated"), "{first}");
    assert!(first.contains("hash=b3:"), "{first}");
    // A marker further down the file is not a marker: otherwise a generated
    // example inside somebody's hand-written prose would claim the file.
    let buried = format!("# Mine\n\n{file}");
    assert_eq!(marker::classify(&buried), RenderState::Foreign);
}

#[test]
fn the_body_is_recoverable_without_the_marker_line() {
    let file = generated(BODY);
    assert_eq!(marker::body_of(&file).as_deref(), Some(BODY));
    assert_eq!(marker::body_of("# not ours\n"), None);
}

/// Rendering the same manifest twice produces the same bytes, or `--dry-run`
/// shows a diff that is not there and `UpdateFile` fires on a no-op.
#[test]
fn wrapping_is_deterministic() {
    assert_eq!(generated(BODY), generated(BODY));
}
