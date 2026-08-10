//! Running `git`, with the environment constructed rather than inherited.
//!
//! `env_clear()` plus an explicit allowlist. Argument filtering alone would be
//! theatre here: `GIT_SSH_COMMAND`, `GIT_EXTERNAL_DIFF` and
//! `GIT_CONFIG_COUNT`/`_KEY`/`_VALUE` all reach the same code paths without
//! appearing in any argument, so an inherited environment bypasses argv checks
//! entirely (ADR-0005 §10 rule 3).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::agents::GitOp;
use crate::detect::DetectError;

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

    /// Spawn `git`, drain both pipes, and enforce a deadline.
    ///
    /// The one place this crate creates a `git` process. Both public entry
    /// points funnel here so `env_clear()` cannot be forgotten on one of them.
    fn spawn(
        &self,
        cwd: &Path,
        args: &[String],
        env: &BTreeMap<OsString, OsString>,
        timeout: Duration,
    ) -> Result<CommandOutput, DetectError> {
        let mut argv = vec!["git".to_owned()];
        argv.extend(args.iter().cloned());

        let mut child = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| DetectError::NotAttempted {
                why: format!("could not run git: {e}"),
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
                        why: format!("waiting for git failed: {e}"),
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
        self.spawn(cwd, &owned, &child_env(), self.timeout)
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
        self.spawn(op.cwd(), &argv, &op_env(op), timeout)
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
}
