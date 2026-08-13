//! The prompts a project defines, read from the directory the agent already
//! reads — and, for the project's own, whether its repository would expose
//! them.
//!
//! Phase 2 of ADR-0010. Phase 1 is the instrument ([`crate::ignore_state`]);
//! phase 3 is the display. **This module produces data and renders nothing**,
//! which is not only layering: §7's display is three facts, and one of them is
//! a label this module deliberately declines to flatten (see [`Exposure`]).
//!
//! # What is read, and what is asked
//!
//! `<project>/.claude/commands` and `<home>/.claude/commands`, both flat into
//! one namespace, with **user-level shadowing project-level** on an identical
//! name (§2, measured as a pair). The invoked name is derived from the path —
//! case preserved, spaces preserved, a directory separator becoming `:`.
//!
//! Exposure is asked of `git` once per project prompt. **N spawns, deliberately
//! (§5b)**: the batched `-n -v` form reads the per-file answer off *stdout*,
//! which would put a state behind a text format, where today `ignored` and
//! `not ignored` are reachable only from exit `0` and exit `1`. That bound is
//! what makes §5's version-dependence cost a diagnostic and never a state, and
//! it is not for sale at 14.5 ms a spawn.
//!
//! # The trap in the obvious implementation
//!
//! **Do not walk this directory with the `ignore` crate.** It is in this
//! workspace, it is what `scan` uses, and it skips gitignored files — which
//! under polarity B is *exactly the set this feature exists to show*. A private
//! prompt is an ignored file. A walker that honours `.gitignore` reports the
//! published ones and silently omits every private one, and the resulting
//! listing looks complete.
//!
//! # An empty listing is not automatically a fact
//!
//! A missing `.claude/commands` is genuinely zero prompts. A directory that
//! could not be read is **not** zero prompts, and flattening the two gives an
//! empty list that reads as *"this project defines no prompts"* — ADR-0002 §7's
//! empty-result-without-a-positive-control, arriving in product code instead of
//! in a search. So each root reports its own [`RootOutcome`], and a read error
//! anywhere in the walk makes the count not a fact.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::exec::ProcessRunner;
use crate::ignore_state::{IgnoreState, check_ignore};

/// The directory Claude Code reads prompts from, relative to either root.
///
/// Two components rather than one string, so the separator is the platform's
/// and never a literal `/` in a path.
const CLAUDE_DIR: &str = ".claude";
const COMMANDS_DIR: &str = "commands";

/// The measured extension, matched **case-sensitively**.
///
/// §2 measured `commands/notmd.txt` as absent from the resolved list, against a
/// positive control of 56 present commands from the same directory. What was
/// **not** measured is `.MD`, and this build does not guess.
///
/// # The gap runs both ways, and the direction this build takes is not the safe
/// one
///
/// Stating only one half of this was the first draft's error, so both are here:
///
/// - **Matching case-insensitively invents a prompt.** On a case-sensitive
///   filesystem, `daily.MD` may be a file Claude Code never loads, and listing
///   it reports a prompt that does not exist.
/// - **Matching case-sensitively omits one.** Windows and default macOS are
///   case-insensitive, so `daily.MD` may genuinely load there — and **under
///   polarity B an unlisted prompt is one whose exposure nobody sees**, which
///   is the failure §5 exists against. This is the direction that matters, and
///   it is the direction this constant takes.
///
/// So this is a gap held open deliberately, not a decision that the risk is
/// gone. It is recorded rather than closed because closing it by assumption is
/// what §1's recorded blank refuses.
///
/// # It costs one run, not a new harness
///
/// The question is *"does Claude Code load `daily.MD`"*, and the instrument
/// already exists: the resolved command list on the `system/init` record of
/// `--output-format stream-json`, with a `.md` file in the same directory as the
/// positive control — the identical shape that established the `notmd.txt` row.
/// ADR-0010's revisit triggers carry it.
const PROMPT_EXT: &str = "md";

/// A bound on how deep the walk goes, so a pathological tree cannot run away.
///
/// §2 measured one level of nesting (`sub/deep.md` → `/sub:deep`). Deeper
/// nesting generalises the same rule and is **not measured**; the bound exists
/// for the walk's safety, not as a claim about the resolver.
const MAX_DEPTH: usize = 32;

/// Which root a prompt came from, and where that root is.
///
/// **The path is always present, whether or not it was asked about.** That is
/// what makes [`Exposure`] a pair rather than a state with a special case: a
/// reader who sees no exposure answer still learns which repository the
/// question belongs to, which is what they act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "root", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PromptRoot {
    /// `<dir>/.claude/commands` — the project under consideration.
    Project { dir: PathBuf },
    /// `<dir>/.claude/commands` — the user's own, shared across all projects.
    User { dir: PathBuf },
}

impl PromptRoot {
    /// A stable identifier, safe to branch on and to print as data.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            PromptRoot::Project { .. } => "project",
            PromptRoot::User { .. } => "user",
        }
    }

    /// The directory the root is anchored at.
    #[must_use]
    pub fn dir(&self) -> &Path {
        match self {
            PromptRoot::Project { dir } | PromptRoot::User { dir } => dir,
        }
    }

    /// The `.claude/commands` directory under this root.
    #[must_use]
    fn commands_dir(&self) -> PathBuf {
        self.dir().join(CLAUDE_DIR).join(COMMANDS_DIR)
    }
}

/// Which root a prompt's exposure question belongs to, and the answer where one
/// was asked for.
///
/// # Why this is a pair and not a fourth [`IgnoreState`] variant
///
/// ADR-0010 §5a. The obvious shape — adding `DifferentRoot { root }` alongside
/// `Ignored` / `NotIgnored` / `Unknown` — carries the root *inside* the variant,
/// so the moment a user-level prompt is asked about its own repository (the
/// dotfiles case, a foreseeable opt-in) the answer becomes `NotIgnored` and
/// **the field that said which root it was about is destroyed**. Under polarity
/// B that renders identically to a project prompt's `NotIgnored`, which reads as
/// *"a `git add -A` here would pick this up"* — false, and in the reassuring
/// direction.
///
/// Keeping the root beside the state makes that opt-in additive: fill the
/// `Option`, change no variant, break no consumer.
///
/// # There is deliberately no `key()` here
///
/// A single label over this pair is the flattening above with a nicer name. A
/// consumer must read both halves, and the type gives it no shortcut — the same
/// technique as the missing `FileOp::Delete` and [`check_ignore`] returning a
/// state rather than a `Result`. Naming the *display* label is phase 3's job
/// (§7), and phase 3 cannot name it without the root in hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Exposure {
    root: PromptRoot,
    state: Option<IgnoreState>,
}

impl Exposure {
    /// A project prompt, asked about against the project root.
    #[must_use]
    pub fn project(dir: PathBuf, state: IgnoreState) -> Self {
        Exposure {
            root: PromptRoot::Project { dir },
            state: Some(state),
        }
    }

    /// A prompt whose exposure question belongs to a different repository, and
    /// which was therefore not asked about.
    ///
    /// **Not an error and not `unknown`.** Nothing failed: git was never
    /// invoked. Asking with `project_dir = <project>` would return
    /// `PathOutsideRepository`, which is correct and reads as a fault (§5a).
    #[must_use]
    pub fn different_root(dir: PathBuf) -> Self {
        Exposure {
            root: PromptRoot::User { dir },
            state: None,
        }
    }

    /// The root the question belongs to. Always present.
    #[must_use]
    pub fn root(&self) -> &PromptRoot {
        &self.root
    }

    /// The answer, where one was asked for.
    #[must_use]
    pub fn state(&self) -> Option<&IgnoreState> {
        self.state.as_ref()
    }

    /// Whether this prompt's exposure was asked about at all.
    ///
    /// Exists so a caller writes `if !exposure.was_asked()` rather than
    /// comparing against a state — the same reason [`IgnoreState::is_known`]
    /// exists one level down.
    #[must_use]
    pub fn was_asked(&self) -> bool {
        self.state.is_some()
    }
}

/// One prompt, as it exists on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Prompt {
    /// The name it is invoked by, derived from the path and **not in the
    /// file** (§7). Case preserved, spaces preserved, separators as `:`.
    pub name: String,
    /// Where the file is.
    pub path: PathBuf,
    /// The path relative to its own `.claude/commands`, which is what the name
    /// was derived from.
    pub relative: PathBuf,
    /// Which repository's question this is, and the answer where asked.
    pub exposure: Exposure,
}

/// A project prompt that is unreachable because a user-level prompt owns its
/// name.
///
/// §6: shadowing is a permanent condition rather than a collision to check
/// once, so it is reported at read time. The shadowed file **is still on disk
/// and still in the repository**, so it keeps its exposure — being unreachable
/// by name does not make it unexposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Shadowed {
    pub prompt: Prompt,
    /// The file that owns the name instead.
    pub shadowed_by: PathBuf,
}

/// What happened when a root's `.claude/commands` was read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RootOutcome {
    /// The directory was read to the end. `count` files became prompts, and
    /// **that number is a fact**.
    Read { count: usize },
    /// There is no such directory. Zero prompts, and that is also a fact.
    Absent,
    /// The walk did not finish, so any prompts found here are **a partial list
    /// that must not be reported as a complete one**.
    ///
    /// **Named for its consequence rather than its cause**, the same way
    /// [`UnknownCause::NoExitCode`](crate::ignore_state::UnknownCause::NoExitCode)
    /// is. There are two producers and they are not one thing: the root's own
    /// `read_dir` failing for a reason that is not absence, and something going
    /// wrong *during* the walk — an unreadable subdirectory, a bad entry, or a
    /// tree nested past [`MAX_DEPTH`]. The cause travels in `why`; the
    /// consequence is identical and it is the consequence a caller acts on.
    ///
    /// **Both producers need their own control.** They are separate branches
    /// and a control for one leaves the other unexercised — found by sabotage,
    /// which is the only reason it is written down here.
    Unreadable { why: String },
}

impl RootOutcome {
    /// A stable identifier, safe to branch on and to print as data.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            RootOutcome::Read { .. } => "read",
            RootOutcome::Absent => "absent",
            RootOutcome::Unreadable { .. } => "unreadable",
        }
    }

    /// Whether the prompts found under this root are the complete set.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !matches!(self, RootOutcome::Unreadable { .. })
    }
}

/// One root, and what reading it produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RootReport {
    pub root: PromptRoot,
    pub outcome: RootOutcome,
}

/// What the base layer knows about the plugin namespace, which is nothing.
///
/// §6: plugin-supplied entries sit in the same flat namespace unprefixed — 19
/// of the 56 in the measured enumeration — and the base layer does not read
/// plugin directories or `enabledPlugins`, because that would be an allowlist
/// of sources and allowlists go stale silently.
///
/// **One variant, on purpose.** `NotAttempted` as a value rather than as a
/// comment forces a consumer to render it; it is ADR-0003's line, one medium
/// over. The enrichment path of §6 adds a second variant here, and
/// `#[non_exhaustive]` is what makes that additive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "plugins", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PluginNamespace {
    /// Not asked. **Not "unshadowed"** — a name reported here can be owned by a
    /// plugin installed later, with nothing in the project changing.
    NotAttempted,
}

/// Every prompt the base layer can see, with resolution applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct PromptListing {
    /// Resolved: one entry per invoked name, user-level having won any
    /// collision. Sorted by name, so a listing is stable across runs.
    pub prompts: Vec<Prompt>,
    /// Project prompts a user-level file made unreachable.
    pub shadowed: Vec<Shadowed>,
    /// One per root looked at, in project-then-user order.
    pub roots: Vec<RootReport>,
    /// Always [`PluginNamespace::NotAttempted`] from this layer.
    pub plugins: PluginNamespace,
}

impl PromptListing {
    /// Whether every root reported a complete read.
    ///
    /// **The question a caller must ask before reading an empty list as an
    /// answer.** Zero prompts with this `true` means the project defines none;
    /// zero prompts with it `false` means vibe could not find out.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.roots.iter().all(|r| r.outcome.is_complete())
    }
}

/// Read both roots, resolve the namespace, and ask about the project's own.
///
/// `user_home` is passed in rather than resolved here so a test can plant one:
/// a harness that read the real `~/.claude` while the subject read a planted
/// one is the harness-versus-subject disagreement (ADR-0002 §7). Callers get
/// the real one from [`user_home`].
///
/// `exec` is asked once per **project** prompt and never for a user-level one
/// (§5a).
#[must_use]
pub fn list_prompts(
    project_dir: &Path,
    user_home: Option<&Path>,
    exec: &dyn ProcessRunner,
) -> PromptListing {
    let project_root = PromptRoot::Project {
        dir: project_dir.to_path_buf(),
    };
    let (project_found, project_outcome) = read_root(&project_root);

    let (user_found, user_report) = match user_home {
        Some(home) => {
            let root = PromptRoot::User {
                dir: home.to_path_buf(),
            };
            let (found, outcome) = read_root(&root);
            (found, Some(RootReport { root, outcome }))
        }
        None => (Vec::new(), None),
    };

    // The project's own, each asked about individually.
    let project_prompts: Vec<Prompt> = project_found
        .into_iter()
        .map(|found| {
            let state = check_ignore(project_dir, &pathspec(&found.relative), exec);
            Prompt {
                name: found.name,
                path: found.path,
                relative: found.relative,
                exposure: Exposure::project(project_dir.to_path_buf(), state),
            }
        })
        .collect();

    // The user's own, never asked about.
    let user_prompts: Vec<Prompt> = user_found
        .into_iter()
        .map(|found| {
            let dir = user_report
                .as_ref()
                .map_or_else(PathBuf::new, |r| r.root.dir().to_path_buf());
            Prompt {
                name: found.name,
                path: found.path,
                relative: found.relative,
                exposure: Exposure::different_root(dir),
            }
        })
        .collect();

    // One flat namespace, and user-level wins (§2, measured as a pair).
    let owners: BTreeMap<&str, &Path> = user_prompts
        .iter()
        .map(|p| (p.name.as_str(), p.path.as_path()))
        .collect();

    let mut shadowed = Vec::new();
    let mut prompts = Vec::new();
    for prompt in project_prompts {
        if let Some(by) = owners.get(prompt.name.as_str()) {
            shadowed.push(Shadowed {
                shadowed_by: by.to_path_buf(),
                prompt,
            });
        } else {
            prompts.push(prompt);
        }
    }
    prompts.extend(user_prompts);
    prompts.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    shadowed.sort_by(|a, b| a.prompt.name.cmp(&b.prompt.name));

    let mut roots = vec![RootReport {
        root: project_root,
        outcome: project_outcome,
    }];
    roots.extend(user_report);

    PromptListing {
        prompts,
        shadowed,
        roots,
        plugins: PluginNamespace::NotAttempted,
    }
}

/// The user's home directory, where `~/.claude/commands` lives.
///
/// `None` where the platform will not say, which is a degradation and not an
/// error: the project's own prompts still list, and the user-level root simply
/// reports nothing rather than being guessed at.
#[must_use]
pub fn user_home() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// A file found under a root, before it has an exposure.
struct Found {
    name: String,
    path: PathBuf,
    relative: PathBuf,
}

/// Walk one root's `.claude/commands`.
fn read_root(root: &PromptRoot) -> (Vec<Found>, RootOutcome) {
    let dir = root.commands_dir();
    let mut found = Vec::new();
    let mut trouble: Option<String> = None;

    match std::fs::read_dir(&dir) {
        Ok(_) => walk(&dir, Path::new(""), 0, &mut found, &mut trouble),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return (found, RootOutcome::Absent);
        }
        Err(err) => {
            return (
                found,
                RootOutcome::Unreadable {
                    why: format!("{}: {err}", dir.display()),
                },
            );
        }
    }

    let outcome = match trouble {
        // A partial walk is not a count. Whatever was found still travels, and
        // the outcome is what stops it being read as the whole set.
        Some(why) => RootOutcome::Unreadable { why },
        None => RootOutcome::Read { count: found.len() },
    };
    (found, outcome)
}

/// Recursive half of [`read_root`].
///
/// **Directory symlinks are not followed.** `DirEntry::file_type` reports a
/// symlink as a symlink rather than as its target, so the `is_dir()` test below
/// declines to descend — which stops a loop, and is the reason the residual in
/// §5's fourth pass is about paths rather than about traversal.
fn walk(
    dir: &Path,
    rel: &Path,
    depth: usize,
    found: &mut Vec<Found>,
    trouble: &mut Option<String>,
) {
    if depth > MAX_DEPTH {
        record(
            trouble,
            format!("{}: nested deeper than {MAX_DEPTH}", dir.display()),
        );
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            record(trouble, format!("{}: {err}", dir.display()));
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                record(trouble, format!("{}: {err}", dir.display()));
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                record(trouble, format!("{}: {err}", entry.path().display()));
                continue;
            }
        };

        let name = entry.file_name();
        let child_rel = rel.join(&name);
        if file_type.is_dir() {
            walk(&entry.path(), &child_rel, depth + 1, found, trouble);
        } else if file_type.is_file() {
            if let Some(name) = invoked_name(&child_rel) {
                found.push(Found {
                    name,
                    path: entry.path(),
                    relative: child_rel,
                });
            }
        }
    }
}

/// Keep the first thing that went wrong, not the last.
///
/// The first is the one nearest the top of the tree and the most likely to
/// explain the rest; a later error is often a consequence of it.
fn record(trouble: &mut Option<String>, why: String) {
    if trouble.is_none() {
        *trouble = Some(why);
    }
}

/// The name a prompt is invoked by, derived from its path under
/// `.claude/commands` (§2).
///
/// Returns `None` for anything that is not a prompt file, which is the
/// `commands/notmd.txt` row of the measured table.
fn invoked_name(rel: &Path) -> Option<String> {
    if rel.extension().and_then(|e| e.to_str()) != Some(PROMPT_EXT) {
        return None;
    }

    let mut parts: Vec<String> = Vec::new();
    for component in rel.with_extension("").components() {
        // Anything that is not an ordinary name — a root, a prefix, `.` or
        // `..` — cannot come from this walk, and a name is not invented for it.
        let Component::Normal(part) = component else {
            return None;
        };
        parts.push(part.to_str()?.to_owned());
    }
    if parts.is_empty() {
        return None;
    }
    // The measured mapping: `sub/deep.md` -> `sub:deep`, case and spaces
    // preserved, nothing else rewritten.
    Some(parts.join(":"))
}

/// The relative path as a pathspec `git` will read the same way on every
/// platform.
///
/// Built with `/` deliberately. A Windows `PathBuf` joins with `\`, and what a
/// backslash pathspec means to `git` is a fact about `git` that this crate has
/// not measured — the same reason `--no-pager` was chosen over an empty
/// `GIT_PAGER` (ADR-0005 §4a). A forward slash needs no such fact.
///
/// It is relative rather than absolute so it cannot acquire a leading `..`,
/// which `git` rejects on the pathspec's *text* even when it resolves back
/// inside the repository (measured, ADR-0010 §5).
fn pathspec(relative: &Path) -> PathBuf {
    let joined: Vec<String> = relative
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    PathBuf::from(format!("{CLAUDE_DIR}/{COMMANDS_DIR}/{}", joined.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ignore_state::UnknownCause;

    fn name(rel: &str) -> Option<String> {
        invoked_name(&PathBuf::from(rel))
    }

    /// §2's measured table, every row.
    ///
    /// The last row is a negative, so it travels with the rows above it as its
    /// positive control — the same discipline §2 used when it recorded the
    /// negative against an enumeration of 56 present commands.
    #[test]
    fn the_invoked_name_is_the_measured_mapping() {
        assert_eq!(name("probe.md").as_deref(), Some("probe"));
        assert_eq!(name("Mixed_Case.md").as_deref(), Some("Mixed_Case"));
        assert_eq!(name("two words.md").as_deref(), Some("two words"));
        assert_eq!(name("sub/deep.md").as_deref(), Some("sub:deep"));
        // The negative, and the four above are what stop it being vacuous.
        assert_eq!(name("notmd.txt"), None);
    }

    /// The extension match is case-sensitive because only `.md` was measured.
    /// Asserted so the choice is visible rather than incidental: a future
    /// measurement of `.MD` changes this line and this test together.
    #[test]
    fn an_uppercase_extension_is_not_assumed_to_be_a_prompt() {
        assert_eq!(name("shouty.MD"), None);
        assert_eq!(name("quiet.md").as_deref(), Some("quiet"));
    }

    /// Deeper nesting generalises the measured one-level rule. Recorded as a
    /// generalisation rather than as a measurement.
    #[test]
    fn nesting_deeper_than_the_measured_level_keeps_joining_with_colons() {
        assert_eq!(name("a/b/c.md").as_deref(), Some("a:b:c"));
    }

    /// The pathspec is what reaches `git`, so its two properties are asserted
    /// where they are decided rather than inferred from a state.
    #[test]
    fn the_pathspec_uses_forward_slashes_and_cannot_climb_out() {
        let spec = pathspec(&PathBuf::from("shared").join("deploy.md"));
        assert_eq!(spec.to_string_lossy(), ".claude/commands/shared/deploy.md");
        assert!(!spec.to_string_lossy().contains(".."));
        // A backslash, written as an escape so the assertion says what it
        // means on a platform whose own separator is the other one.
        assert!(!spec.to_string_lossy().contains('\u{5c}'));
    }

    // --- the pair that keeps a user-level answer from reading as a project's -

    #[test]
    fn a_project_exposure_carries_a_state_and_a_different_root_does_not() {
        let asked = Exposure::project(PathBuf::from("/p"), IgnoreState::Ignored);
        assert!(asked.was_asked());
        assert_eq!(asked.state(), Some(&IgnoreState::Ignored));
        assert_eq!(asked.root().key(), "project");

        let unasked = Exposure::different_root(PathBuf::from("/home/u"));
        assert!(!unasked.was_asked());
        assert_eq!(unasked.state(), None);
        assert_eq!(unasked.root().key(), "user");
        // The datum the label carries: where to look instead.
        assert_eq!(unasked.root().dir(), Path::new("/home/u"));
    }

    /// `different_root` is **not** an unknown, and the distinction is the whole
    /// of §5a: nothing failed, so there is no cause and none is invented.
    #[test]
    fn different_root_is_not_an_unknown_state() {
        let unasked = Exposure::different_root(PathBuf::from("/home/u"));
        assert_eq!(unasked.state(), None);
        // The partner: what an unknown actually looks like, through the same
        // accessor. One is an absent answer; the other is a failed one.
        let unknown = Exposure::project(
            PathBuf::from("/p"),
            IgnoreState::Unknown {
                cause: UnknownCause::NotARepository,
            },
        );
        assert!(unknown.was_asked());
        assert!(!unknown.state().expect("asked").is_known());
    }

    // --- completeness -------------------------------------------------------

    #[test]
    fn only_an_unreadable_root_makes_a_listing_incomplete() {
        assert!(RootOutcome::Read { count: 0 }.is_complete());
        assert!(RootOutcome::Absent.is_complete());
        assert!(
            !RootOutcome::Unreadable {
                why: "denied".to_owned()
            }
            .is_complete()
        );
    }

    #[test]
    fn the_outcome_keys_are_three_and_distinct() {
        assert_eq!(RootOutcome::Read { count: 1 }.key(), "read");
        assert_eq!(RootOutcome::Absent.key(), "absent");
        assert_eq!(
            RootOutcome::Unreadable { why: String::new() }.key(),
            "unreadable"
        );
    }

    /// The listing serializes as data a frontend can switch on, with the root
    /// beside the state rather than folded into it.
    #[test]
    fn the_exposure_serializes_with_its_root_beside_its_state() {
        let json = serde_json::to_value(Exposure::different_root(PathBuf::from("/home/u")))
            .expect("serialize");
        assert_eq!(json["root"]["root"], "user");
        assert!(json["state"].is_null(), "{json}");

        let json = serde_json::to_value(Exposure::project(
            PathBuf::from("/p"),
            IgnoreState::NotIgnored,
        ))
        .expect("serialize");
        assert_eq!(json["root"]["root"], "project");
        assert_eq!(json["state"]["state"], "not_ignored");
    }

    /// §6, as a value rather than as a comment.
    #[test]
    fn the_plugin_namespace_is_not_attempted() {
        let json = serde_json::to_value(PluginNamespace::NotAttempted).expect("serialize");
        assert_eq!(json["plugins"], "not_attempted");
    }
}
