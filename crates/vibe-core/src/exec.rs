//! Running `git` and `gh`, with the environment constructed rather than
//! inherited.
//!
//! `env_clear()` plus an explicit allowlist. Argument filtering alone would be
//! theatre here: `GIT_SSH_COMMAND`, `GIT_EXTERNAL_DIFF` and
//! `GIT_CONFIG_COUNT`/`_KEY`/`_VALUE` all reach the same code paths without
//! appearing in any argument, so an inherited environment bypasses argv checks
//! entirely (ADR-0005 §10 rule 3).
//!
//! Both programs funnel through one `spawn`, so `env_clear()` cannot be
//! forgotten on one of them — and neither can the timeout, the pipe draining,
//! or the argument check.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::detect::DetectError;
use crate::gh::{GH_PROGRAM, GhOp};
use crate::git::GitOp;

/// The captured result of a subprocess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// The exact argv, so it can be cited as evidence.
    pub argv: Vec<String>,
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    #[must_use]
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// Stdout with trailing newlines removed.
    #[must_use]
    pub fn trimmed(&self) -> &str {
        self.stdout.trim_end_matches(['\n', '\r'])
    }
}

pub trait ProcessRunner: Send + Sync + std::fmt::Debug {
    fn run_git(&self, cwd: &Path, args: &[&str]) -> Result<CommandOutput, DetectError>;
    /// Whether `git` can be run at all. A missing `git` is a degradation, not
    /// an error: everything that does not need it still works.
    fn git_available(&self) -> bool;

    /// Run one of the closed store operations.
    ///
    /// Separate from [`ProcessRunner::run_git`] on purpose. `run_git` takes a
    /// free `&[&str]`, which is safe only because every caller is a detector
    /// passing argv this crate wrote as a literal. The store's `clone` is the
    /// first invocation carrying a string the *user* chose, and the answer to
    /// that is the closed enum, not a longer filter — so it gets an entry point
    /// that cannot be handed an argument vector at all (ADR-0005 §10 rule 1).
    ///
    /// It also gets a different timeout: a clone is minutes where a detector
    /// query is budgeted in milliseconds.
    fn run_git_op(&self, op: &GitOp) -> Result<CommandOutput, DetectError>;

    /// Run one of the closed `gh` operations.
    ///
    /// Its own entry point rather than a `program` field on the git one.
    /// ADR-0005 §10 rule 1 keys the allowlist on the `(program, subcommand)`
    /// pair, and the two programs have different pairs, different environments
    /// and different argument allowlists. A shared entry point taking a program
    /// name would be the `{git, gh}` program allowlist the ADR rejected,
    /// rebuilt one layer down.
    fn run_gh_op(&self, op: &GhOp) -> Result<CommandOutput, DetectError>;

    /// Whether `gh` can be run at all.
    ///
    /// Probed through the **same constructed environment** the real invocation
    /// uses. An earlier version of this probe span `gh` with the inherited
    /// environment while `run_gh_op` would have cleared it, which is ADR-0002
    /// §7's harness-versus-subject disagreement built into the product rather
    /// than into a test: the two would eventually answer differently, and the
    /// difference would surface as "vibe said gh was there and then could not
    /// run it".
    fn gh_available(&self) -> bool;
}

/// Arguments that turn a read-only `git` query into arbitrary execution.
///
/// Detection only ever passes argv this crate wrote, so in practice these can
/// arrive only inside a *value*. That is exactly why the check is worth having:
/// it catches the future call site that threads a branch name or a remote into
/// a position that turns out to be flag-parsed.
const FORBIDDEN_ARGS: &[&str] = &[
    "-c",
    "--exec-path",
    "--upload-pack",
    "--receive-pack",
    "--config-env",
    "--namespace",
    "--git-dir",
    "--work-tree",
];

fn reject_dangerous_args(args: &[&str]) -> Result<(), DetectError> {
    for arg in args {
        let head = arg.split('=').next().unwrap_or(arg);
        if FORBIDDEN_ARGS.contains(&head) {
            return Err(DetectError::NotAttempted {
                why: format!("refusing to run git with `{arg}`"),
            });
        }
    }
    Ok(())
}

/// The same check, exposed so [`crate::agents::GitOp`]'s tests can assert that
/// no variant is *capable* of emitting a forbidden argument.
///
/// That direction matters more than the runtime check does. The runtime check
/// catches a bad argument on its way out; this asserts the enum has nowhere to
/// put one, which is the difference between rejecting the dangerous thing and
/// making it unrepresentable.
#[cfg(test)]
pub(crate) fn assert_argv_is_clean(args: &[&str]) -> Result<(), DetectError> {
    reject_dangerous_args(args)
}

/// The `(program, subcommand)` pairs reachable through `gh`.
///
/// ADR-0005 §10 rule 1 keys the allowlist on the pair, and this is the pair
/// list for `gh`. `alias` and `extension` are absent, which is the whole point:
/// they execute arbitrary binaries by design.
const GH_ALLOWED_PAIRS: &[[&str; 2]] = &[["repo", "create"]];

/// Every flag any [`GhOp`] may emit before the `--` separator.
///
/// An **allowlist**, not a denylist, and the difference is the `ext::` lesson:
/// nobody writes down a flag they have not heard of. `gh repo create` alone has
/// `--template`, `--clone`, `--homepage` and more; enumerating what we *do*
/// emit is a list of four strings that a reviewer can check against
/// [`GhOp::argv`], where enumerating what we must not emit is a list that grows
/// every `gh` release without anyone noticing.
const GH_ALLOWED_FLAGS: &[&str] = &["--source=.", "--push", "--private", "--public"];

/// Reject any `gh` argv that is not one of the constructed shapes.
///
/// Belt and braces to [`GhOp`]'s closed enum, in the direction that fails
/// closed: the check runs to the first `--` and requires every element to be
/// allowlisted, so a future variant threading a user string into a
/// pre-separator slot is refused rather than admitted.
///
/// Elements *after* the separator are data. That is the deliberate divergence
/// from [`reject_dangerous_args`], which scans everything: there the guarded
/// slot only ever carries paths this crate constructs, and here it carries the
/// project name the user chose. Refusing to create a repository named `alias`
/// would be a validation rule wearing a containment rule's clothes — and
/// `cobra` resolves subcommands from the leading positionals before flag
/// parsing, so nothing after the separator can become one.
fn reject_dangerous_gh_args(args: &[&str]) -> Result<(), DetectError> {
    let refused = |what: String| DetectError::NotAttempted { why: what };

    let pair = match args {
        [a, b, ..] => [*a, *b],
        _ => return Err(refused("refusing to run gh with no subcommand".to_owned())),
    };
    if !GH_ALLOWED_PAIRS.contains(&pair) {
        return Err(refused(format!(
            "refusing to run `gh {} {}`: not an allowlisted subcommand",
            pair[0], pair[1]
        )));
    }

    for arg in &args[2..] {
        // The separator ends the flags. What follows is a value.
        if *arg == "--" {
            return Ok(());
        }
        if !GH_ALLOWED_FLAGS.contains(arg) {
            return Err(refused(format!("refusing to run gh with `{arg}`")));
        }
    }
    // No separator at all means every element was checked against the flag
    // allowlist, which is the conservative direction and not an oversight.
    Ok(())
}

/// Exposed so [`crate::gh::GhOp`]'s tests can assert both directions: that no
/// variant is capable of producing a refused argv, and — the half that makes
/// the first non-vacuous — that `alias set` and `extension install` really are
/// refused when handed to the checker directly.
#[cfg(test)]
pub(crate) fn assert_gh_argv_is_clean(args: &[&str]) -> Result<(), DetectError> {
    reject_dangerous_gh_args(args)
}

/// Environment variables a child process legitimately needs.
///
/// Everything else is dropped. In particular there is no `GIT_*` or `GH_*`
/// passthrough of any kind.
fn child_env() -> BTreeMap<OsString, OsString> {
    let mut env = BTreeMap::new();
    let wanted: &[&str] = &[
        "PATH",
        "HOME",
        // Windows needs these to resolve executables and locate per-user config.
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "PATHEXT",
        "COMSPEC",
    ];
    for key in wanted {
        if let Some(val) = std::env::var_os(key) {
            env.insert(OsString::from(key), val);
        }
    }

    // Set positively, not merely left unset: clearing the environment stops an
    // inherited hostile value, but /etc/gitconfig is read regardless and can
    // define aliases and core.sshCommand just as a repo-local config can.
    env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    // Deterministic, parseable output regardless of the user's locale.
    env.insert("LC_ALL".into(), "C".into());
    // Never block waiting for a credential prompt.
    env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    env
}

/// The child environment for one store operation.
///
/// Extends [`child_env`] with the ssh agent socket, and **only for the ops that
/// reach the network**. This is ADR-0005 §10 rule 3a's reasoning applied to a
/// variable the ADR does not name: `SSH_AUTH_SOCK` is a credential channel, not
/// a configuration setting — anything that can see the variable can ask the
/// agent to sign with the user's keys. A `git reset --hard` in a local
/// directory has no use for that, so it does not get it.
///
/// Without this, cloning a private store over `ssh://` works only with a
/// passphrase-less key that `git` finds via `HOME`. That is a real degradation
/// and forwarding the socket to `clone` and `fetch` is the narrowest fix.
fn op_env(op: &GitOp) -> BTreeMap<OsString, OsString> {
    let mut env = child_env();
    if op.needs_network() {
        for key in ["SSH_AUTH_SOCK", "SSH_AGENT_PID"] {
            if let Some(val) = std::env::var_os(key) {
                env.insert(OsString::from(key), val);
            }
        }
    }
    // No `GITHUB_TOKEN` branch, deliberately: no store op returns true from
    // `needs_credential`, and `git` cannot consume the token anyway. See the
    // method's docs.
    debug_assert!(!op.needs_credential());
    env
}

/// The child environment for one `gh` operation.
///
/// [`child_env`] plus four things `gh` needs and one it must not get.
///
/// **`XDG_CONFIG_HOME` is forwarded.** `gh` looks for its configuration — and
/// therefore its credential — in `$XDG_CONFIG_HOME/gh`, falling back to
/// `~/.config/gh`. Dropping it would leave a user who moved their config
/// directory silently unauthenticated. It is admitted on exactly the ground
/// rule 3 admits `HOME`: a program cannot find its own configuration without
/// being told where it is, and it grants nothing `HOME` does not already grant,
/// since both name a directory whose contents `gh` reads. What makes that
/// acceptable rather than a hole is ADR-0008 §6, verified rather than assumed:
/// a per-user `gh` config does not redirect `gh repo create`.
///
/// **`GH_PAGER` is set to blank.** `gh` pipes output through a pager named by
/// its own config file, which is a *command* reachable through the config
/// directory above — the `ext::` shape in a new place. Blank disables paging
/// outright, so there is no pager to name. This is the same reasoning that sets
/// `GIT_CONFIG_NOSYSTEM=1` positively rather than trusting the environment to
/// be empty.
///
/// **`SSH_AUTH_SOCK` is forwarded, because this op pushes.** A `gh` configured
/// with `git_protocol: ssh` wires an `ssh://` remote and pushes over it. Same
/// rule as [`op_env`]: an agent socket is a credential channel, so it belongs
/// in the environment of the ops that can use it and nowhere else.
///
/// **No `GH_TOKEN`/`GITHUB_TOKEN`, deliberately** (ADR-0008 §5). The cost is
/// real and is reported rather than hidden — see [`GhOp::needs_credential`].
fn gh_env(op: &GhOp) -> BTreeMap<OsString, OsString> {
    let mut env = child_env();
    // `gh` shells out to `git` for --source and --push, so the git hardening in
    // `child_env` covers that child too.
    for key in ["XDG_CONFIG_HOME"] {
        if let Some(val) = std::env::var_os(key) {
            env.insert(OsString::from(key), val);
        }
    }
    if op.needs_network() {
        for key in ["SSH_AUTH_SOCK", "SSH_AGENT_PID"] {
            if let Some(val) = std::env::var_os(key) {
                env.insert(OsString::from(key), val);
            }
        }
    }
    // Blank means "no pager" to `gh`, so a config-named pager binary is never
    // spawned.
    env.insert("GH_PAGER".into(), String::new().into());
    // Never block waiting for an answer nobody is there to give.
    env.insert("GH_PROMPT_DISABLED".into(), "1".into());
    // No version-check request, and no notice interleaved with the output.
    env.insert("GH_NO_UPDATE_NOTIFIER".into(), "1".into());
    // Parseable output regardless of what the terminal claims to be.
    env.insert("NO_COLOR".into(), "1".into());
    debug_assert!(!op.needs_credential());
    env
}

#[derive(Debug, Clone)]
pub struct SystemRunner {
    timeout: Duration,
    /// The budget for an operation that talks to a remote. Separate because a
    /// clone of a real repository takes longer than every other `git` call in
    /// this crate put together, and applying the detector budget to it would
    /// make `vibe agents update` fail on any store worth having.
    network_timeout: Duration,
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self {
            // Generous for a local query, short enough that one wedged
            // repository cannot consume the scan budget.
            timeout: Duration::from_millis(1500),
            network_timeout: Duration::from_secs(300),
        }
    }
}

impl SystemRunner {
    #[must_use]
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_network_timeout(mut self, timeout: Duration) -> Self {
        self.network_timeout = timeout;
        self
    }

    /// Spawn a subprocess, drain both pipes, and enforce a deadline.
    ///
    /// The one place this crate creates a process. Every public entry point
    /// funnels here so `env_clear()` cannot be forgotten on one of them.
    ///
    /// `program` is a `&'static str` from a caller in this module, never a
    /// value from a plan or a config: this parameter selects between two
    /// compiled-in constants and is not a program allowlist, which ADR-0005 §10
    /// says closes nothing.
    fn spawn(
        &self,
        program: &'static str,
        cwd: &Path,
        args: &[String],
        env: &BTreeMap<OsString, OsString>,
        timeout: Duration,
    ) -> Result<CommandOutput, DetectError> {
        let mut argv = vec![program.to_owned()];
        argv.extend(args.iter().cloned());

        let mut child = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| DetectError::NotAttempted {
                why: format!("could not run {program}: {e}"),
            })?;

        // Drain both pipes on their own threads. Polling `try_wait` while a
        // child fills a pipe buffer deadlocks: the child blocks on write, the
        // parent waits for exit, and neither moves.
        let mut out = child.stdout.take();
        let mut err = child.stderr.take();
        let out_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(p) = out.as_mut() {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        });
        let err_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(p) = err.as_mut() {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        });

        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(e) => {
                    return Err(DetectError::NotAttempted {
                        why: format!("waiting for {program} failed: {e}"),
                    });
                }
            }
        };

        let stdout = out_handle.join().unwrap_or_default();
        let stderr = err_handle.join().unwrap_or_default();

        let Some(status) = status else {
            return Err(DetectError::Timeout);
        };

        Ok(CommandOutput {
            argv,
            status: status.code(),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }
}

impl ProcessRunner for SystemRunner {
    fn git_available(&self) -> bool {
        Command::new("git")
            .arg("--version")
            .env_clear()
            .envs(child_env())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn run_git(&self, cwd: &Path, args: &[&str]) -> Result<CommandOutput, DetectError> {
        reject_dangerous_args(args)?;
        let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        self.spawn("git", cwd, &owned, &child_env(), self.timeout)
    }

    fn run_git_op(&self, op: &GitOp) -> Result<CommandOutput, DetectError> {
        let argv = op.argv();
        // Belt and braces. Rule 1 already makes a forbidden argument
        // unrepresentable, so this can only fire if a future variant threads a
        // user string into a slot that turns out to be flag-parsed — which is
        // precisely the case ADR-0005 §10 rule 2 says the check exists for.
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        reject_dangerous_args(&refs)?;

        let timeout = if op.needs_network() {
            self.network_timeout
        } else {
            self.timeout
        };
        self.spawn("git", op.cwd(), &argv, &op_env(op), timeout)
    }

    fn run_gh_op(&self, op: &GhOp) -> Result<CommandOutput, DetectError> {
        let argv = op.argv();
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        reject_dangerous_gh_args(&refs)?;
        // Always the network budget: every variant creates a repository and
        // pushes to it.
        self.spawn(
            GH_PROGRAM,
            op.cwd(),
            &argv,
            &gh_env(op),
            self.network_timeout,
        )
    }

    fn gh_available(&self) -> bool {
        // The same environment `run_gh_op` builds, minus the per-op additions
        // that `--version` cannot use. Probing through a *different*
        // environment is the harness-versus-subject mistake (ADR-0002 §7)
        // committed in shipping code, where nothing would catch it.
        Command::new(GH_PROGRAM)
            .arg("--version")
            .env_clear()
            .envs(child_env())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

/// A runner that refuses everything, for tests and for the case where `git` is
/// not installed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRunner;

impl ProcessRunner for NoRunner {
    fn git_available(&self) -> bool {
        false
    }

    fn run_git(&self, _cwd: &Path, _args: &[&str]) -> Result<CommandOutput, DetectError> {
        Err(DetectError::NotAttempted {
            why: "git is not available".to_owned(),
        })
    }

    fn run_git_op(&self, _op: &GitOp) -> Result<CommandOutput, DetectError> {
        Err(DetectError::NotAttempted {
            why: "git is not available".to_owned(),
        })
    }

    fn run_gh_op(&self, _op: &GhOp) -> Result<CommandOutput, DetectError> {
        Err(DetectError::NotAttempted {
            why: "gh is not available".to_owned(),
        })
    }

    fn gh_available(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_git_arguments_are_refused() {
        for bad in [
            "-c",
            "--exec-path=/tmp/evil",
            "--upload-pack=sh",
            "--config-env=x",
            "--git-dir",
        ] {
            let err = reject_dangerous_args(&[bad]).expect_err("{bad} should be refused");
            assert!(matches!(err, DetectError::NotAttempted { .. }), "{bad}");
        }
    }

    #[test]
    fn ordinary_git_arguments_are_allowed() {
        reject_dangerous_args(&["remote", "get-url", "origin"]).unwrap();
        reject_dangerous_args(&["log", "-1", "--format=%cI"]).unwrap();
        reject_dangerous_args(&["status", "--porcelain"]).unwrap();
    }

    #[test]
    fn the_child_environment_is_constructed_not_inherited() {
        let env = child_env();
        assert_eq!(
            env.get(&OsString::from("GIT_CONFIG_NOSYSTEM"))
                .map(|v| v.to_string_lossy().into_owned()),
            Some("1".to_owned())
        );
        assert!(env.contains_key(&OsString::from("PATH")));

        // Nothing GIT_*-ish is forwarded except the hardening flags we set.
        let forwarded: Vec<_> = env
            .keys()
            .map(|k| k.to_string_lossy().into_owned())
            .filter(|k| k.starts_with("GIT_") || k.starts_with("GH_"))
            .collect();
        assert_eq!(
            forwarded,
            vec!["GIT_CONFIG_NOSYSTEM", "GIT_TERMINAL_PROMPT"]
        );
    }

    fn a_gh_op() -> GhOp {
        GhOp::RepoCreate {
            cwd: std::path::PathBuf::from("/d/proj"),
            name: "demo".to_owned(),
            visibility: crate::gh::RepoVisibility::Private,
        }
    }

    /// The `gh` environment is the git one plus named additions, and **no
    /// token**.
    ///
    /// ADR-0005 §10 rule 3a named `GhOp::RepoCreate` as the op that would carry
    /// `GITHUB_TOKEN`; ADR-0008 §5 answers that it carries none. This is that
    /// answer as an assertion, so a future edit that adds one has to change a
    /// test that says why it should not.
    #[test]
    fn the_gh_environment_carries_no_credential_and_no_pager() {
        let env = gh_env(&a_gh_op());
        let keys: Vec<String> = env
            .keys()
            .map(|k| k.to_string_lossy().into_owned())
            .collect();

        for forbidden in ["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"] {
            assert!(
                !keys.iter().any(|k| k == forbidden),
                "{forbidden} reached gh"
            );
        }
        // Nothing token-shaped under any other name either.
        assert!(
            !keys.iter().any(|k| k.contains("TOKEN")),
            "a credential-shaped variable reached gh: {keys:?}"
        );

        // Blank, not absent: absent means `gh` falls back to its config file,
        // which can name an arbitrary program to pipe output through.
        assert_eq!(
            env.get(&OsString::from("GH_PAGER"))
                .map(OsString::as_os_str),
            Some(std::ffi::OsStr::new("")),
            "a config-named pager is reachable again"
        );

        // Every GH_-prefixed key is one we set positively. A forwarded one
        // would mean the constructed environment had become a filtered one.
        let gh_keys: Vec<&String> = keys.iter().filter(|k| k.starts_with("GH_")).collect();
        assert_eq!(
            gh_keys,
            vec!["GH_NO_UPDATE_NOTIFIER", "GH_PAGER", "GH_PROMPT_DISABLED"]
        );
    }

    /// The paired half of the allowlist tests in [`crate::gh`]: the checker
    /// must accept the shape the enum actually produces. A checker that
    /// refused everything would satisfy every "hostile argv is refused"
    /// assertion and break the product.
    #[test]
    fn the_gh_checker_accepts_what_the_enum_constructs_and_refuses_the_rest() {
        let argv = a_gh_op().argv();
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        reject_dangerous_gh_args(&refs).expect("the constructed argv must pass");

        assert!(reject_dangerous_gh_args(&["alias", "set", "x", "!sh"]).is_err());
        assert!(reject_dangerous_gh_args(&["extension", "install", "o/e"]).is_err());
        assert!(reject_dangerous_gh_args(&["repo"]).is_err());
        assert!(reject_dangerous_gh_args(&[]).is_err());
    }
}
