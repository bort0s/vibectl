//! Controls for phase 3's display.
//!
//! **Every guard below was sabotaged and observed red before it was committed**
//! (ADR-0002 §7), and each assertion that something is *absent* travels with the
//! case where it must be *present* — a renderer that printed nothing at all
//! satisfies every one-sided half perfectly.
//!
//! # Why the runner is scripted here and delegating in phase 2
//!
//! `prompts_listing.rs` wraps a real [`SystemRunner`] because there the subject
//! **is** the instrument, and a stub would have turned its assertions into
//! assertions about the stub. Here the subject is the renderer and the listing
//! is its *input*, so a scripted runner is the fixture rather than a substitute
//! for the thing under test — and it buys the four display states
//! deterministically on a machine with no `git`, which is what lets the
//! rendering be asserted on all three runners.

use std::path::{Path, PathBuf};

use vibe_core::detect::DetectError;
use vibe_core::exec::CommandOutput;
use vibe_core::prompts::list_prompts;
use vibe_core::{GitOp, ProcessRunner, PromptListing, UnknownCause};

use super::{Remedy, exposure_label, remedy_for, write_prompt_list_human};

// --- fixtures --------------------------------------------------------------

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("has a parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write");
}

/// A `check-ignore` whose answer is keyed on the file name.
///
/// Deterministic and git-free. The names are chosen to say what they buy:
/// `secret` is ignored, `boom` cannot run git at all, `norepo` is a `128` with
/// git's own not-a-repository wording, `dead` returns no exit code, and anything
/// else is a plain exit `1`.
#[derive(Debug)]
struct Scripted;

fn output(status: Option<i32>, stderr: &str) -> CommandOutput {
    CommandOutput {
        argv: vec!["git".to_owned(), "check-ignore".to_owned()],
        status,
        stdout: String::new(),
        stderr: stderr.to_owned(),
    }
}

impl ProcessRunner for Scripted {
    fn run_git(&self, _cwd: &Path, _args: &[&str]) -> Result<CommandOutput, DetectError> {
        Err(DetectError::NotAttempted {
            why: "the scripted runner answers check-ignore only".to_owned(),
        })
    }

    fn git_available(&self) -> bool {
        true
    }

    fn run_git_op(&self, op: &GitOp) -> Result<CommandOutput, DetectError> {
        let GitOp::CheckIgnore { path, .. } = op else {
            return Err(DetectError::NotAttempted {
                why: "the scripted runner answers check-ignore only".to_owned(),
            });
        };
        let name = path.to_string_lossy().replace('\\', "/");
        if name.contains("secret") {
            Ok(output(Some(0), ""))
        } else if name.contains("boom") {
            Err(DetectError::NotAttempted {
                why: "could not run git: program not found".to_owned(),
            })
        } else if name.contains("norepo") {
            Ok(output(
                Some(128),
                "fatal: not a git repository (or any of the parent directories): .git\n",
            ))
        } else if name.contains("dead") {
            Ok(output(None, ""))
        } else {
            Ok(output(Some(1), ""))
        }
    }

    fn run_gh_op(&self, _op: &vibe_core::GhOp) -> Result<CommandOutput, DetectError> {
        Err(DetectError::NotAttempted {
            why: "no gh here".to_owned(),
        })
    }

    fn gh_available(&self) -> bool {
        false
    }
}

fn render(listing: &PromptListing) -> String {
    let mut buf = Vec::new();
    write_prompt_list_human(&mut buf, listing).expect("writing to a Vec cannot fail");
    String::from_utf8(buf).expect("utf-8")
}

/// A project with one prompt per display state, and a user home beside it.
fn listing_with_every_state(tmp: &Path) -> PromptListing {
    let project = tmp.join("proj");
    let home = tmp.join("home");
    write(&project.join(".claude/commands/secret.md"), "private\n");
    write(&project.join(".claude/commands/open.md"), "public\n");
    write(&project.join(".claude/commands/boom.md"), "no git\n");
    write(&home.join(".claude/commands/mine.md"), "user level\n");
    list_prompts(&project, Some(&home), &Scripted)
}

/// The root's own `read_dir` fails: `.claude/commands` is a file.
fn root_unreadable(dir: &Path) {
    write(&dir.join(".claude/commands"), "not a directory\n");
}

/// The walk does not finish: a tree past the bound. **The second producer**, and
/// the one phase 2's sabotage found uncovered — a control for the first leaves
/// this branch unexercised, because a `commands` that is a file returns before
/// the walk begins.
fn walk_unfinished(dir: &Path) {
    write(&dir.join(".claude/commands/top.md"), "shallow\n");
    let mut deep = dir.join(".claude/commands");
    for i in 0..40 {
        deep = deep.join(format!("d{i}"));
    }
    write(&deep.join("buried.md"), "too deep\n");
}

fn all_causes() -> Vec<UnknownCause> {
    vec![
        UnknownCause::GitNotRun { why: "w".into() },
        UnknownCause::NotARepository,
        UnknownCause::PathOutsideRepository,
        UnknownCause::TimedOut,
        UnknownCause::NoExitCode,
        UnknownCause::Unrecognised {
            status: 128,
            stderr: "s".into(),
        },
    ]
}

// --- the four display states ------------------------------------------------

/// Each state gets its own word, and — the half that matters — **no state
/// borrows another's**.
///
/// The one-sided version of this passes against a renderer that prints
/// `not ignored` on every row, which is the exact defect §5 exists against.
#[test]
fn the_four_display_states_each_get_their_own_word() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let listing = listing_with_every_state(tmp.path());
    let text = render(&listing);

    let row = |name: &str| -> String {
        text.lines()
            .find(|l| l.starts_with(name))
            .unwrap_or_else(|| panic!("no row for {name}\n{text}"))
            .to_owned()
    };

    assert!(row("secret").contains("ignored"), "{text}");
    assert!(row("open").contains("not ignored"), "{text}");
    assert!(row("boom").contains("unknown"), "{text}");
    assert!(row("mine").contains("not asked"), "{text}");

    // The pairing. `ignored` is a substring of `not ignored`, so the ignored row
    // is checked for the *absence* of the negation rather than the presence of
    // the word — the assertion above cannot tell them apart on its own.
    assert!(!row("secret").contains("not ignored"), "{text}");
    assert!(!row("open").contains("unknown"), "{text}");
    assert!(!row("boom").contains("not ignored"), "{text}");
    assert!(!row("mine").contains("unknown"), "{text}");
}

/// The two words that must not be the same word.
///
/// The plugin footer and a user-level row are both "vibe did not ask", and
/// rendering them identically lets a reader discharge the row along with the
/// footer. The row carries exposure; the footer does not.
#[test]
fn a_not_asked_row_does_not_reuse_the_plugin_namespaces_words() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let text = render(&listing_with_every_state(tmp.path()));
    let row = text
        .lines()
        .find(|l| l.starts_with("mine"))
        .expect("the user-level row");

    assert!(row.contains("not asked"), "{text}");
    assert!(!row.to_lowercase().contains("not attempted"), "{text}");
    assert!(!row.to_lowercase().contains("plugin"), "{text}");
    // And the footer is still there saying its own thing, so this is not passing
    // because the plugin note vanished.
    assert!(
        text.contains("Plugin-supplied prompts were not checked"),
        "{text}"
    );
}

/// §5a's datum: the label says nothing about where to look, so the root must.
/// **Found by sabotage, and the repair is the point.**
///
/// The first version asserted `text.contains(<home>)`. That observable has
/// **two producers** — the note, and the FILE column of every user-level row,
/// which already spells the home path out. Deleting the note left this test
/// green, because the assertion was being satisfied by the row it was not about.
///
/// That is ADR-0002 §7's *count the branches that produce the observable you are
/// asserting on* landing inside a control that read as complete, one level up
/// from where phase 2 found it. The fix is to assert on the note's **own line**,
/// so only one producer can satisfy it.
#[test]
fn the_not_asked_rows_say_which_root_the_question_belongs_to() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let text = render(&listing_with_every_state(tmp.path()));

    let note = text
        .lines()
        .find(|l| l.contains("belongs to another repository"))
        .unwrap_or_else(|| panic!("no root note at all\n{text}"));
    assert!(
        note.contains(&home.display().to_string()),
        "the note does not name the root, so `not asked` says where to look nowhere\n{text}"
    );

    // Paired: a listing with no user-level prompts must not print the note, or
    // the assertion above is satisfied by a renderer that always prints it.
    let project = tmp.path().join("solo");
    write(&project.join(".claude/commands/open.md"), "public\n");
    let solo = list_prompts(&project, None, &Scripted);
    assert!(!render(&solo).contains("belongs to another repository"));
}

// --- the unknown cause reaching the reader ----------------------------------

/// **The frontend half of ADR-0001 §4's chain, for `UnknownCause`.**
///
/// New variant → core's `identity` match stops compiling → author adds the pair
/// and extends `ALL_KEYS` → the length check here reds → author writes the
/// remedy. Driven by `ALL_KEYS`, and no stronger than that list is.
#[test]
fn every_unknown_cause_has_a_remedy_entry() {
    let all = all_causes();
    assert_eq!(
        all.len(),
        UnknownCause::ALL_KEYS.len(),
        "a cause was added to core and this list did not follow"
    );
    for cause in all {
        assert_ne!(
            remedy_for(cause.code()),
            Remedy::NoEntry,
            "`{}` has no entry in the remedy catalogue",
            cause.code()
        );
    }
}

/// The paired half: a code this build does not know **must** reach `NoEntry`, or
/// the test above is asserting against a catalogue that cannot miss.
#[test]
fn a_cause_this_build_does_not_know_reaches_the_no_entry_arm() {
    assert_eq!(
        remedy_for("VIBE_S_IGNORE_SOMETHING_A_LATER_BUILD_ADDED"),
        Remedy::NoEntry
    );
}

/// An unknown row carries **core's description and this crate's remedy**, and
/// they are two lines rather than one.
///
/// The description is core's because `UnknownCause` has no `Display` and
/// `to_wire().message` is the only English that exists for it. The remedy is
/// ours because ADR-0010 §5 splits these causes *by remedy*.
#[test]
fn an_unknown_row_carries_cores_description_and_this_crates_remedy() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let listing = listing_with_every_state(tmp.path());
    let text = render(&listing);

    // **Core's description, taken from core rather than retyped.** The first
    // draft of this assertion built the expected `UnknownCause` by hand and got
    // it wrong: `classify` fills `why` from `DetectError`'s `Display`, so the
    // real cause carries a `not attempted: ` prefix a hand-built one does not.
    // Reading the cause out of the listing is the `GhOp::argv` discipline —
    // assert against the producer's own output, never against a copy of it.
    let cause = listing
        .prompts
        .iter()
        .find(|p| p.name == "boom")
        .and_then(|p| match p.exposure.state() {
            Some(vibe_core::IgnoreState::Unknown { cause }) => Some(cause),
            _ => None,
        })
        .expect("boom is the unknown row");
    assert!(text.contains(&cause.to_wire().message), "{text}");
    // This crate's remedy, which core does not carry and must not.
    assert!(text.contains("install git"), "{text}");

    // Paired: a row that is *not* unknown gets neither line.
    let project = tmp.path().join("clean");
    write(&project.join(".claude/commands/open.md"), "public\n");
    let clean = render(&list_prompts(&project, None, &Scripted));
    assert!(!clean.contains("install git"), "{clean}");
}

/// A cause with no remedy says so, rather than printing its code where a
/// sentence belongs (ADR-0001 §4) or borrowing a neighbour's advice.
#[test]
fn a_cause_with_no_remedy_says_there_is_no_action_rather_than_inventing_one() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("proj");
    write(&project.join(".claude/commands/dead.md"), "no exit code\n");
    let text = render(&list_prompts(&project, None, &Scripted));

    assert!(text.contains("no action here"), "{text}");
    assert!(
        !text.contains("VIBE_S_IGNORE"),
        "a code stood where a sentence belongs\n{text}"
    );
    // Not another cause's remedy.
    assert!(!text.contains("install git"), "{text}");
    assert!(!text.contains("git init"), "{text}");
}

// --- is_complete, per branch ------------------------------------------------

/// **The constraint-5 gate, in its most direct form.**
///
/// Both halves in one test because they are one decision: the sentence is
/// available when the read was complete and forbidden when it was not.
#[test]
fn no_prompts_is_only_stated_when_every_root_was_read_to_the_end() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(empty.join(".claude/commands")).expect("mkdir");
    let complete = render(&list_prompts(&empty, None, &Scripted));
    assert!(
        complete.contains("This project defines no prompts."),
        "{complete}"
    );

    let broken = tmp.path().join("broken");
    root_unreadable(&broken);
    let incomplete = render(&list_prompts(&broken, None, &Scripted));
    assert!(
        !incomplete.contains("This project defines no prompts."),
        "an unreadable directory was reported as a project with no prompts, which is \
         the reading that says all clear\n{incomplete}"
    );
    assert!(
        incomplete.contains("not the same as there being none"),
        "{incomplete}"
    );
}

/// The count is a fact only when the walk finished.
#[test]
fn the_total_is_printed_only_when_the_listing_is_complete() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let ok = tmp.path().join("ok");
    write(&ok.join(".claude/commands/open.md"), "public\n");
    let complete = render(&list_prompts(&ok, None, &Scripted));
    assert!(complete.contains("prompt(s)."), "{complete}");

    let deep = tmp.path().join("deep");
    walk_unfinished(&deep);
    let partial = render(&list_prompts(&deep, None, &Scripted));
    assert!(
        !partial.contains("prompt(s)."),
        "a total was printed over a partial walk\n{partial}"
    );
    assert!(partial.contains("This list is partial"), "{partial}");
    // What was found still travels — partial is not nothing.
    assert!(partial.contains("top"), "{partial}");
}

/// **The four-producer control, one case per branch.**
///
/// `is_complete()` is false from two roots × two `Unreadable` producers, and the
/// two *roots* have different consequences — missing prompts versus unreliable
/// shadowing. One sentence for both is the collapse phase 2's fifth sabotage
/// found, one level up in the display.
#[test]
fn the_two_roots_fail_differently_and_do_not_read_the_same() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Branch 1: project root, its own read_dir.
    let a = tmp.path().join("a");
    root_unreadable(&a.join("proj"));
    write(&a.join("home/.claude/commands/mine.md"), "fine\n");
    let text = render(&list_prompts(
        &a.join("proj"),
        Some(&a.join("home")),
        &Scripted,
    ));
    assert!(text.contains("Prompts may be missing"), "{text}");
    assert!(!text.contains("may be shadowed"), "{text}");

    // Branch 2: project root, mid-walk.
    let b = tmp.path().join("b");
    walk_unfinished(&b.join("proj"));
    write(&b.join("home/.claude/commands/mine.md"), "fine\n");
    let text = render(&list_prompts(
        &b.join("proj"),
        Some(&b.join("home")),
        &Scripted,
    ));
    assert!(text.contains("Prompts may be missing"), "{text}");
    assert!(!text.contains("may be shadowed"), "{text}");

    // Branch 3: user root, its own read_dir.
    let c = tmp.path().join("c");
    write(&c.join("proj/.claude/commands/open.md"), "public\n");
    root_unreadable(&c.join("home"));
    let text = render(&list_prompts(
        &c.join("proj"),
        Some(&c.join("home")),
        &Scripted,
    ));
    assert!(text.contains("may be shadowed"), "{text}");
    assert!(!text.contains("Prompts may be missing"), "{text}");

    // Branch 4: user root, mid-walk.
    let d = tmp.path().join("d");
    write(&d.join("proj/.claude/commands/open.md"), "public\n");
    walk_unfinished(&d.join("home"));
    let text = render(&list_prompts(
        &d.join("proj"),
        Some(&d.join("home")),
        &Scripted,
    ));
    assert!(text.contains("may be shadowed"), "{text}");
    assert!(!text.contains("Prompts may be missing"), "{text}");

    // The pairing for all four: with both roots readable, neither sentence
    // appears. Without this, a renderer that always printed both would pass the
    // presence halves above and fail nothing.
    let e = tmp.path().join("e");
    write(&e.join("proj/.claude/commands/open.md"), "public\n");
    write(&e.join("home/.claude/commands/mine.md"), "fine\n");
    let text = render(&list_prompts(
        &e.join("proj"),
        Some(&e.join("home")),
        &Scripted,
    ));
    assert!(!text.contains("Prompts may be missing"), "{text}");
    assert!(!text.contains("may be shadowed"), "{text}");
}

// --- identity: a name shown is not a name resolved --------------------------

/// A shadowed prompt is listed apart from the reachable ones, says which file
/// the name actually runs, **and keeps its exposure**.
///
/// The last clause is the trap: unreachable by name is not unexposed. The file
/// is still in the repository and a `git add -A` still picks it up.
#[test]
fn a_shadowed_prompt_names_its_owner_and_keeps_its_exposure() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("proj");
    let home = tmp.path().join("home");
    // `open` collides, and is not ignored under the scripted runner — so the
    // exposure it keeps is the one that matters under polarity B.
    write(&project.join(".claude/commands/open.md"), "project copy\n");
    write(&home.join(".claude/commands/open.md"), "user copy\n");
    let listing = list_prompts(&project, Some(&home), &Scripted);
    let text = render(&listing);

    assert_eq!(listing.shadowed.len(), 1, "{listing:?}");
    assert!(text.contains("Not reachable by name"), "{text}");
    assert!(text.contains("the name runs"), "{text}");
    // The exposure survives into the shadowed section.
    let shadow_block = text
        .split("Not reachable by name")
        .nth(1)
        .expect("the shadowed section");
    assert!(
        shadow_block.contains("not ignored"),
        "a shadowed prompt lost its exposure, so an exposed file reads as harmless\n{text}"
    );

    // Paired: remove the user-level file and the section must disappear, or the
    // assertions above pass against a renderer that always prints it.
    std::fs::remove_file(home.join(".claude/commands/open.md")).expect("rm");
    let text = render(&list_prompts(&project, Some(&home), &Scripted));
    assert!(!text.contains("Not reachable by name"), "{text}");
}

/// §6's `NotAttempted`, and the property that makes it worth a line:
/// **it is printed even when there is nothing to print it about.**
#[test]
fn the_plugin_note_is_unconditional_and_reads_as_neither_fine_nor_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(empty.join(".claude/commands")).expect("mkdir");
    let text = render(&list_prompts(&empty, None, &Scripted));

    assert!(
        text.contains("Plugin-supplied prompts were not checked"),
        "{text}"
    );
    // Not "fine": it must say a plugin can own a name.
    assert!(text.contains("can own any of these names"), "{text}");
    // Not an error: nothing failed, so no error vocabulary.
    let lower = text.to_lowercase();
    assert!(!lower.contains("error"), "{text}");
    assert!(!lower.contains("warning"), "{text}");
    assert!(!lower.contains("failed"), "{text}");
}

// --- the wildcard that must not borrow a word -------------------------------

/// The three known states render as themselves, and the fallback arm names
/// itself rather than picking one of them.
///
/// `IgnoreState` is `#[non_exhaustive]`, so a state from a later core cannot be
/// constructed here and the fallback is asserted through `exposure_label`
/// directly — the same split `unranked_severity_label` uses, and for the same
/// reason: the arm that reaches it needs a variant that cannot exist yet, and
/// the body has no such requirement.
#[test]
fn a_state_this_build_cannot_place_does_not_borrow_one_of_the_three_words() {
    use vibe_core::IgnoreState;
    use vibe_core::prompts::Exposure;

    let p = PathBuf::from("/p");
    assert_eq!(
        exposure_label(&Exposure::project(p.clone(), IgnoreState::Ignored)),
        "ignored"
    );
    assert_eq!(
        exposure_label(&Exposure::project(p.clone(), IgnoreState::NotIgnored)),
        "not ignored"
    );
    assert_eq!(
        exposure_label(&Exposure::project(
            p,
            IgnoreState::Unknown {
                cause: UnknownCause::NoExitCode
            }
        )),
        "unknown"
    );
    assert_eq!(
        exposure_label(&Exposure::different_root(PathBuf::from("/home"))),
        "not asked"
    );
}
