//! A tripwire on the one input ADR-0008 §9's revisit trigger depends on.
//!
//! §9 declines to build a per-control mutation gate today, and records a
//! trigger for revisiting: **a seventh integration-test target gated on a
//! `VIBE_REQUIRE_*` variable.** The argument for that trigger is that it is an
//! *event producing a diff someone reviews*, unlike the one it replaced —
//! *"when the controls outgrow what a reviewer checks by eye"* — which cannot
//! fire, because the reviewer who can no longer check by eye is exactly the one
//! not noticing.
//!
//! **The number was being maintained by hand, and that reintroduces the failure
//! the trigger was designed against.** It went from three to four inside a
//! single commit window during ADR-0010 phase 2 — written, then invalidated by
//! the same change before it was pushed. A trigger whose input drifts in minutes
//! cannot be relied on to fire, which is the *"reviewer can no longer check by
//! eye"* shape arriving through the back door.
//!
//! So the count is **derived** here rather than written down, and the check is a
//! gate rather than a command someone must remember: adding a seventh control
//! turns this red in the ordinary test job, where somebody is already looking.
//! That is the `VIBE_REQUIRE_GH` shape (ADR-0002 §7) applied to a trigger — the
//! result carries the proof, so no second channel has to be read.
//!
//! # Why this file excludes itself by path and not by watching its own words
//!
//! The first version assembled the marker with `concat!` so the file would not
//! contain the literal it searches for, and **the prose above put it back three
//! times** — the self-exclusion assertion caught it on the first run. That is
//! the useful half: a file whose subject *is* a marker will name that marker,
//! and a rule requiring it not to is a rule that will be broken by the next
//! person writing a comment.
//!
//! So the exclusion is structural. The marker below is spelled plainly, this
//! target is dropped from the corpus by name, and the assertion that used to
//! police the spelling now guards the exclusion instead. If the file is ever
//! renamed, the exclusion stops matching and the count rises by one — which
//! fires the gate **early** rather than never, and early is the direction a
//! trigger may fail in.
//!
//! # This file is not itself a control
//!
//! It asserts nothing about `git`, `gh`, or containment, and it is deliberately
//! not gated on any of the variables it counts — it must run everywhere,
//! including on a machine with neither tool.

use std::path::{Path, PathBuf};

/// The gate. Raise it only by deciding §9's question, never to make a red go
/// away.
///
/// **WHAT IT DOES NOT MEASURE.** *Added 2026-08-19.* This counts the
/// **skip-path hazard**: controls that depend on an external tool and can
/// silently skip when it is missing. It has never counted *how many controls
/// exist*, and two rounds of controls landing outside it made that look like a
/// fault in the gate rather than a fault in the reading. **Do not read 5 of 7
/// as "controls are stable."** The total is derived and printed beside it by
/// `the_total_control_count_is_reported_beside_the_gated_one`; only this one
/// is a gate.
const TRIGGER_AT: usize = 7;

/// What identifies a control: the prefix of the variables that turn a missing
/// tool into a failure rather than a skip.
const MARKER: &str = "VIBE_REQUIRE_";

/// This target, which counts the others and must never count itself.
const SELF: &str = "control_inventory.rs";

/// Every `crates/*/tests/*.rs` in the workspace, except this one.
fn integration_test_targets() -> Vec<PathBuf> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits at <workspace>/crates/<name>")
        .join("crates");

    let mut targets = Vec::new();
    for crate_dir in std::fs::read_dir(&crates).expect("crates/ is readable") {
        let tests = crate_dir.expect("entry").path().join("tests");
        let Ok(entries) = std::fs::read_dir(&tests) else {
            continue; // a crate with no integration tests
        };
        for entry in entries {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|e| e.to_str()) == Some("rs")
                && path.file_name().and_then(|n| n.to_str()) != Some(SELF)
            {
                targets.push(path);
            }
        }
    }
    targets.sort();
    targets
}

/// **The trigger's input, derived — with positive controls in the same
/// invocation.**
///
/// The count is an empty-result risk in disguise: a wrong workspace root, a
/// changed layout or a renamed variable all produce **zero**, and a gate
/// asserting *"fewer than seven"* passes on zero perfectly. So the assertion is
/// many-sided, per ADR-0002 §7 — an empty result with no positive control is a
/// skipped test wearing a green tick, and here the green would mean the trigger
/// had silently ceased to exist.
#[test]
fn the_number_of_require_gated_controls_is_below_the_revisit_trigger() {
    let targets = integration_test_targets();

    // Positive control one: the search found the corpus at all.
    assert!(
        targets.len() > 5,
        "found only {} integration-test targets, so the workspace root or the \
         layout is not what this search assumes and the count below is not a \
         count",
        targets.len()
    );

    // Positive control two: this target really was excluded, rather than the
    // exclusion silently matching nothing after a rename.
    assert!(
        !targets.iter().any(|p| p.ends_with(SELF)),
        "the inventory is in its own corpus, so every number it reports is off \
         by one"
    );

    let gated: Vec<&PathBuf> = targets
        .iter()
        .filter(|path| {
            std::fs::read_to_string(path)
                .expect("a test target is readable")
                .contains(MARKER)
        })
        .collect();

    // Positive control three, and the one that matters most: the marker still
    // matches something. A renamed variable would take this count to zero and
    // the gate would report green forever.
    assert!(
        !gated.is_empty(),
        "no test target mentions {MARKER}. Either the variables were renamed — \
         in which case this file and ADR-0008 §9 both need updating — or this \
         search is blind. Both are findings; neither is a green."
    );

    assert!(
        gated.len() < TRIGGER_AT,
        "ADR-0008 §9's revisit trigger has fired: {} controls are gated on a \
         {MARKER}* variable, and the trigger is {TRIGGER_AT}. That section \
         declines a per-control mutation gate on the grounds that a small number \
         of controls and a reviewer holding the whole argument is enough. \
         Re-decide it — do not raise this number to make the red go away.\n{}",
        gated.len(),
        gated
            .iter()
            .map(|p| format!("  {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// **The gate's numerator and the number of controls have come apart, and this
/// prints both so the divergence is visible where the gate is.**
///
/// *Added 2026-08-19.* ADR-0008 §9's trigger counts **integration-test targets
/// gated on a `VIBE_REQUIRE_*` variable**, and that marker has one job: turn a
/// *missing external tool* into a failure instead of a skip. Two rounds running,
/// real controls have landed — the reader's damaged-file behaviour, the
/// cold-start measurement, the settings edit, the structural write-path guards —
/// and the gated count has not moved, because **none of them needs `git` or
/// `gh`, so none of them has a skip path to close.** Gating them would be
/// wearing the marker rather than using it.
///
/// So the proxy has come loose from the thing the trigger was reaching for, and
/// *"is a reviewer still holding the whole argument?"* is no longer answered by
/// the number the gate watches. **That is a definition change, which is not
/// this file's to make.** What this file can stop is the divergence being
/// invisible: the total is derived and printed beside the gated count, in the
/// same invocation, so a round that adds five controls and moves the gate by
/// zero says so in CI rather than in a report.
///
/// The total is **derived, never written down** — the same rule the gated count
/// already follows, and for the same reason: a number maintained by hand drifts
/// inside a single commit window.
#[test]
fn the_total_control_count_is_reported_beside_the_gated_one() {
    let targets = integration_test_targets();
    assert!(
        targets.len() > 5,
        "the corpus search found {} targets, so this is not a count",
        targets.len()
    );

    let mut total = 0usize;
    let mut gated_targets = 0usize;
    for path in &targets {
        let src = std::fs::read_to_string(path).expect("a test target is readable");
        total += src
            .lines()
            .filter(|l| l.trim_start().starts_with("#[test]"))
            .count();
        if src.contains(MARKER) {
            gated_targets += 1;
        }
    }

    // The premise: `#[test]` still identifies a control here. A harness change
    // would take this to zero and the print below would read as "no controls".
    assert!(
        total > 50,
        "found only {total} `#[test]` items across {} targets, so this is \
         counting something other than controls",
        targets.len()
    );

    println!(
        "controls: {total} `#[test]` items across {} integration targets; \
         {gated_targets} of those targets are {MARKER}-gated, and ADR-0008 §9's \
         trigger is {TRIGGER_AT} gated targets. The two numbers measure \
         different things and only the second is a gate.",
        targets.len()
    );
}
