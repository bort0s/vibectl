//! The filesystem layer, against a real `git` and a real directory tree.
//!
//! Phase 2 of ADR-0010. `src/prompts.rs`'s own tests cover the name derivation
//! and the type shapes, which need neither. This file covers what only a real
//! fixture can establish: that a private prompt is *listed* rather than walked
//! past, that the exposure a listing reports flips when the rule under it
//! changes, that user-level shadowing is what §2 measured, and that a root
//! which could not be read is not reported as a project with no prompts.
//!
//! # Every control here is paired
//!
//! ADR-0010 §10, and ADR-0002 §7's rule against one-sided controls. An
//! implementation that asked nothing, or shadowed nothing, or called every root
//! unreadable, would satisfy each single-sided half perfectly:
//!
//! | Control | Partner |
//! | --- | --- |
//! | private prompt → `ignored` | rule broken → `not ignored`, same file |
//! | user file present → user wins | same file removed → project wins |
//! | user-level prompt is not asked about | project prompt in the same listing *is* |
//! | unreadable root → not complete | readable root → `Read`, and absent → `Absent` |
//! | `notmd.txt` is not a prompt | `daily.md` beside it is |

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use vibe_core::exec::{CommandOutput, ProcessRunner, SystemRunner};
use vibe_core::git::GitOp;
use vibe_core::ignore_state::IgnoreState;
use vibe_core::prompts::{PromptListing, RootOutcome, list_prompts};

/// Whether these controls can run here — and a hard failure where they must.
///
/// The `VIBE_REQUIRE_GIT` shape, identical to `ignore_state_git.rs`. See that
/// file's guard for what the green step does and does not prove; the same
/// narrowing applies here unchanged, and it is stated once there rather than
/// restated here where it could drift.
fn git_available() -> bool {
    let present = Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        present || std::env::var_os("VIBE_REQUIRE_GIT").is_none(),
        "VIBE_REQUIRE_GIT is set but `git` is not on PATH. These controls are \
         only meaningful where git exists, so refusing to run is reported as a \
         failure rather than a skip (ADR-0002 §7)."
    );
    present
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(path, contents).expect("write");
}

/// Polarity B, exactly as §4 measured it: the bare directory is private and
/// `shared/` is published. The order of these four lines is the whole
/// mechanism — the naive negation does nothing.
const POLARITY_B: &str =
    ".claude/*\n!.claude/commands/\n.claude/commands/*\n!.claude/commands/shared/\n";

/// A project with three private prompts and one published one.
///
/// **The fixture's own `git init` does not run on the product's budget.**
/// `SystemRunner::default()` allows a detector query 1500 ms, which is right
/// for the subject and wrong for setup: a cold start made this `init` exceed it
/// once here, and a fixture that times out reports as a failure of the thing
/// under test. That is the harness producing a finding about itself
/// (ADR-0002 §7). The subject below keeps the default runner, because the
/// budget *is* part of what is being tested there.
fn a_project(root: &Path) {
    SystemRunner::with_timeout(Duration::from_secs(30))
        .run_git_op(&GitOp::Init {
            cwd: root.to_path_buf(),
        })
        .expect("git init runs");
    write(&root.join(".gitignore"), POLARITY_B);
    write(&root.join(".claude/commands/daily.md"), "the daily one\n");
    write(&root.join(".claude/commands/Mixed_Case.md"), "case\n");
    write(&root.join(".claude/commands/two words.md"), "spaces\n");
    write(&root.join(".claude/commands/shared/deploy.md"), "shared\n");
}

fn named<'a>(listing: &'a PromptListing, name: &str) -> &'a vibe_core::prompts::Prompt {
    listing
        .prompts
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no prompt named {name:?}; listing has {:?}",
                listing.prompts.iter().map(|p| &p.name).collect::<Vec<_>>()
            )
        })
}

fn state_of(listing: &PromptListing, name: &str) -> IgnoreState {
    named(listing, name)
        .exposure
        .state()
        .unwrap_or_else(|| panic!("{name} was never asked about"))
        .clone()
}

// --- the mechanism control -------------------------------------------------

/// **A private prompt is listed, and breaking the rule under it flips what the
/// listing says.**
///
/// Two failures at once, and they are different failures. A walker that
/// honoured `.gitignore` — the `ignore` crate is in this workspace and is what
/// `scan` uses — would omit every private prompt and produce a listing that
/// looks complete. A listing that kept them but reported a stale state would
/// show them and lie about them. The first half of this test catches the
/// omission, the second catches the stale state.
#[test]
fn private_prompts_are_listed_and_the_state_flips_with_the_rule() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    a_project(root);
    let exec = SystemRunner::default();

    let before = list_prompts(root, None, &exec);

    // Every prompt is present, including the three the repository excludes.
    let mut names: Vec<&str> = before.prompts.iter().map(|p| p.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["Mixed_Case", "daily", "shared:deploy", "two words"],
        "a private prompt went missing from the listing"
    );

    // And polarity B is what the listing reports: private by default, the
    // `shared/` one published.
    assert_eq!(state_of(&before, "daily"), IgnoreState::Ignored);
    assert_eq!(state_of(&before, "Mixed_Case"), IgnoreState::Ignored);
    assert_eq!(state_of(&before, "two words"), IgnoreState::Ignored);
    assert_eq!(state_of(&before, "shared:deploy"), IgnoreState::NotIgnored);

    // The only change: the rule that made them private.
    write(&root.join(".gitignore"), "# nothing is excluded any more\n");

    let after = list_prompts(root, None, &exec);
    assert_eq!(
        state_of(&after, "daily"),
        IgnoreState::NotIgnored,
        "the listing reported a state that did not follow the rule under it"
    );
    assert_eq!(state_of(&after, "shared:deploy"), IgnoreState::NotIgnored);
}

// --- the shadowing pair ----------------------------------------------------

/// **User-level shadows project-level, and removing only that file gives the
/// name back.**
///
/// The shape §2 measured, turned into a control. The partner is what makes it
/// mean anything: an implementation that never resolved a collision, or one
/// that always preferred the user's, would each satisfy one half.
///
/// The shadowed project file keeps its exposure, which is not incidental —
/// being unreachable by name does not make it unexposed, and under polarity B
/// a shadowed prompt sitting in `shared/` is still published.
#[test]
fn a_user_prompt_shadows_the_project_one_and_removing_it_gives_the_name_back() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("project");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&project).expect("mkdir");
    a_project(&project);
    let collide = home.join(".claude/commands/daily.md");
    write(&collide, "the user's daily\n");
    let exec = SystemRunner::default();

    let shadowing = list_prompts(&project, Some(&home), &exec);
    assert_eq!(
        named(&shadowing, "daily").path,
        collide,
        "the project's file won a name the user's file owns"
    );
    assert_eq!(shadowing.shadowed.len(), 1, "{:?}", shadowing.shadowed);
    let hidden = &shadowing.shadowed[0];
    assert_eq!(hidden.prompt.name, "daily");
    assert_eq!(
        hidden.prompt.path,
        project.join(".claude/commands/daily.md")
    );
    assert_eq!(hidden.shadowed_by, collide);
    // Unreachable by name, still in the repository, still ignored.
    assert_eq!(
        hidden.prompt.exposure.state(),
        Some(&IgnoreState::Ignored),
        "a shadowed prompt lost its exposure"
    );

    // The partner: remove that one file and nothing else.
    std::fs::remove_file(&collide).expect("remove");

    let resolved = list_prompts(&project, Some(&home), &exec);
    assert_eq!(
        named(&resolved, "daily").path,
        project.join(".claude/commands/daily.md"),
        "the project's file did not get its name back"
    );
    assert!(resolved.shadowed.is_empty(), "{:?}", resolved.shadowed);
}

// --- the different-root pair ----------------------------------------------

/// **A user-level prompt is not asked about, and a project prompt in the same
/// listing is.**
///
/// ADR-0010 §5a. The pair is the point: a build that asked nothing at all would
/// satisfy "the user's prompt was not asked about" perfectly, and the green
/// would mean nothing.
///
/// The count is asserted through a runner that records every op, so this is the
/// mechanism and not an inference from the result — *"exposure is computed only
/// for project prompts"* is a claim about invocations, and the result alone
/// cannot distinguish "never asked" from "asked and discarded".
#[test]
fn only_project_prompts_are_asked_about_and_the_count_proves_it() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("project");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&project).expect("mkdir");
    a_project(&project);
    write(&home.join(".claude/commands/personal.md"), "mine\n");

    let exec = Recording::new();
    let listing = list_prompts(&project, Some(&home), &exec);

    // The user's prompt: listed, rooted at the user's directory, not asked.
    let personal = named(&listing, "personal");
    assert!(
        !personal.exposure.was_asked(),
        "a user-level prompt was given an exposure computed against the project"
    );
    assert_eq!(personal.exposure.root().key(), "user");
    assert_eq!(personal.exposure.root().dir(), home);

    // The partner, in the same listing: a project prompt with a real answer.
    let daily = named(&listing, "daily");
    assert!(daily.exposure.was_asked());
    assert_eq!(daily.exposure.root().key(), "project");
    assert_eq!(daily.exposure.state(), Some(&IgnoreState::Ignored));

    // And the invocations themselves: four project prompts, four asks, and
    // nothing asked about the fifth.
    let asked = exec.paths();
    assert_eq!(asked.len(), 4, "asked about {asked:?}");
    for path in &asked {
        let text = path.to_string_lossy().replace('\\', "/");
        assert!(
            text.starts_with(".claude/commands/"),
            "a pathspec left the project's commands directory: {text}"
        );
        assert!(!text.contains(".."), "a pathspec acquired a `..`: {text}");
        assert!(
            !text.contains("personal"),
            "the user's prompt was asked about: {text}"
        );
    }
}

// --- the unreadable pair ---------------------------------------------------

/// **A root that could not be read is not a project with no prompts.**
///
/// Three outcomes, and the two that mean *zero prompts* must not be reachable
/// from the one that means *vibe could not find out*. This is ADR-0002 §7's
/// empty-result rule in product code: an empty listing with no way to tell the
/// cases apart is the reassuring reading of a failure.
///
/// **What this control exercises, and what it does not.** The failure is
/// constructed by putting a *file* where `.claude/commands` should be, so
/// `read_dir` returns an error that is not `NotFound` — a real, portable,
/// constructed precondition. A **permission** failure reaches the same branch
/// through a different errno and is deliberately not exercised: `chmod 000`
/// does not deny root, CI runners differ on who they run as, and a control
/// whose firing depends on that is one that can go quiet without failing
/// (ADR-0002 §7). The class is "read_dir failed for a reason other than
/// absence"; this control covers it by one instance and says so.
#[test]
fn an_unreadable_root_is_not_the_same_as_an_absent_one() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let exec = SystemRunner::default();

    // 1. Read: the directory is there and was walked to the end.
    let ok = tmp.path().join("ok");
    std::fs::create_dir_all(&ok).expect("mkdir");
    a_project(&ok);
    let read = list_prompts(&ok, None, &exec);
    assert!(read.is_complete());
    assert_eq!(read.roots[0].outcome, RootOutcome::Read { count: 4 });

    // 2. Absent: no such directory. Zero prompts, and that is a fact.
    let bare = tmp.path().join("bare");
    std::fs::create_dir_all(&bare).expect("mkdir");
    let absent = list_prompts(&bare, None, &exec);
    assert!(absent.prompts.is_empty());
    assert_eq!(absent.roots[0].outcome, RootOutcome::Absent);
    assert!(
        absent.is_complete(),
        "a project that genuinely defines no prompts was reported as unknown"
    );

    // 3. Empty: the directory is there and holds nothing. Also a fact, and
    //    distinguishable from both neighbours.
    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(empty.join(".claude/commands")).expect("mkdir");
    let empty = list_prompts(&empty, None, &exec);
    assert!(empty.prompts.is_empty());
    assert_eq!(empty.roots[0].outcome, RootOutcome::Read { count: 0 });
    assert!(empty.is_complete());

    // 4. Unreadable: zero prompts, and that is NOT a fact.
    let broken = tmp.path().join("broken");
    std::fs::create_dir_all(broken.join(".claude")).expect("mkdir");
    write(&broken.join(".claude/commands"), "not a directory\n");
    let unreadable = list_prompts(&broken, None, &exec);
    assert!(unreadable.prompts.is_empty());
    assert_eq!(unreadable.roots[0].outcome.key(), "unreadable");
    assert!(
        !unreadable.is_complete(),
        "an unreadable root reported the same emptiness as a project with no \
         prompts, which is the reading that says 'all clear'"
    );
}

/// **The second producer of `Unreadable`, which the test above does not
/// reach.**
///
/// Found by sabotage rather than by reading the code: breaking the mid-walk
/// branch left the test above **green**, because a `.claude/commands` that is a
/// file fails at the root's own `read_dir` and returns before the walk begins.
/// Two branches, one control — the unreached-guard rule (ADR-0002 §7) inside a
/// control that otherwise looked complete.
///
/// A tree nested past the walk's depth bound reaches the other branch, and it
/// is portable and deterministic where a permission failure is neither. The
/// pairing is the same shape: past the bound the listing is incomplete, and a
/// shallow tree through the identical call is complete.
#[test]
fn a_walk_that_does_not_finish_is_also_not_a_complete_listing() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    a_project(root);
    let exec = SystemRunner::default();

    // The partner first, so a fixture that broke the walk outright would fail
    // here rather than pass the assertion below for the wrong reason.
    let shallow = list_prompts(root, None, &exec);
    assert!(shallow.is_complete());
    assert_eq!(shallow.roots[0].outcome, RootOutcome::Read { count: 4 });

    // 40 levels, comfortably past the bound.
    let mut deep = root.join(".claude/commands");
    for i in 0..40 {
        deep = deep.join(format!("d{i}"));
    }
    write(&deep.join("buried.md"), "too deep\n");

    let listing = list_prompts(root, None, &exec);
    assert_eq!(
        listing.roots[0].outcome.key(),
        "unreadable",
        "{:?}",
        listing.roots[0].outcome
    );
    assert!(
        !listing.is_complete(),
        "a walk that stopped early reported the same completeness as one that \
         finished, so a prompt below the bound is invisible and looks absent"
    );
    // What it did find still travels — a partial list is not nothing, it is
    // just not a count.
    assert!(
        listing.prompts.iter().any(|p| p.name == "daily"),
        "the partial listing threw away what it had found"
    );
}

/// A file that is not a prompt is not listed — **with the positive control in
/// the same fixture**, because a listing that dropped everything would satisfy
/// the negative half perfectly (§2's own discipline, and ADR-0002 §7's).
#[test]
fn a_non_md_file_is_not_a_prompt_and_the_md_beside_it_is() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    a_project(root);
    write(&root.join(".claude/commands/notmd.txt"), "not a prompt\n");
    write(&root.join(".claude/commands/README"), "no extension\n");

    let listing = list_prompts(root, None, &SystemRunner::default());
    let names: Vec<&str> = listing.prompts.iter().map(|p| p.name.as_str()).collect();

    assert!(!names.contains(&"notmd"), "{names:?}");
    assert!(!names.contains(&"notmd.txt"), "{names:?}");
    assert!(!names.contains(&"README"), "{names:?}");
    // The control: the same directory, the same walk, a file that IS a prompt.
    assert!(names.contains(&"daily"), "{names:?}");
    assert_eq!(listing.roots[0].outcome, RootOutcome::Read { count: 4 });
}

// --- a runner that records what it was asked ------------------------------

/// A [`SystemRunner`] that keeps every `check-ignore` path it was handed.
///
/// Delegating rather than stubbing, deliberately: a stub would make the states
/// above assertions about the stub. The recording is additive, so the subject
/// is still the real invocation through the real constructed environment.
#[derive(Debug)]
struct Recording {
    inner: SystemRunner,
    seen: Mutex<Vec<PathBuf>>,
}

impl Recording {
    fn new() -> Self {
        Recording {
            inner: SystemRunner::default(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn paths(&self) -> Vec<PathBuf> {
        self.seen.lock().expect("not poisoned").clone()
    }
}

impl ProcessRunner for Recording {
    fn run_git(
        &self,
        cwd: &Path,
        args: &[&str],
    ) -> Result<CommandOutput, vibe_core::detect::DetectError> {
        self.inner.run_git(cwd, args)
    }

    fn git_available(&self) -> bool {
        self.inner.git_available()
    }

    fn run_git_op(&self, op: &GitOp) -> Result<CommandOutput, vibe_core::detect::DetectError> {
        if let GitOp::CheckIgnore { path, .. } = op {
            self.seen.lock().expect("not poisoned").push(path.clone());
        }
        self.inner.run_git_op(op)
    }

    fn run_gh_op(
        &self,
        op: &vibe_core::gh::GhOp,
    ) -> Result<CommandOutput, vibe_core::detect::DetectError> {
        self.inner.run_gh_op(op)
    }

    fn gh_available(&self) -> bool {
        self.inner.gh_available()
    }
}
