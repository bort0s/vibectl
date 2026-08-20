//! `vibe monitor install` and `vibe monitor status`.
//!
//! Kept out of [`crate::monitor`] deliberately: that module's contract is the
//! **hook** path, where Claude Code reads the exit status and `2` is a blocking
//! error fed back to the agent. Nothing here runs as somebody else's child, so
//! `vibectl`'s ordinary exit contract (ADR-0002 §3) applies unmodified — and
//! putting the two in one file would leave a module doc that is true of half
//! its contents.
//!
//! # Everything ambient is resolved HERE, in vibe's own process
//!
//! The home directory, the executable and the sink root. ADR-0011 §7a requires
//! the sink to be resolved on this side and *declared* in argv, because the
//! hook runs in a child whose environment §3 measured as contaminated by its
//! parent's. The same argument covers the other two, and
//! [`vibe_core::monitor::install`] takes all three as parameters so no control
//! below has to depend on the real user's machine.

use std::io::Write;
use std::path::PathBuf;

use vibe_core::error::CoreError;
use vibe_core::monitor::{InstallOutcome, InstallRequest, InstallState, WriterIdentity, install};

use crate::cli::{MonitorInstallArgs, MonitorStatusArgs};
use crate::exit::Exit;
use crate::output;
use crate::output::pretty;
use crate::reporter;

/// Where the sink lives, and the home the settings file hangs off.
///
/// **`None` is a degradation with a name, not a panic.** The platform declining
/// to say where home is means install cannot run — but it must say *that*,
/// rather than falling back to a guessed path and writing a hook config
/// somewhere nobody looks.
///
/// # LIMIT: this resolution is not overridable by environment, and the cause is
/// UNKNOWN
///
/// *Measured 2026-08-20 on Windows 10 Pro 19045, `directories` 6.0.0.*
///
/// `vibe monitor status` resolves home correctly under the unmodified
/// environment. Setting `USERPROFILE` to a directory that **exists** makes
/// [`vibe_core::prompts::user_home`] return `None`, and the command then
/// reports that the platform will not say where home is. Reproduced four ways —
/// forward slashes and backslashes, from Git Bash and from PowerShell — and it
/// was `None` every time.
///
/// **The reason this is recorded is that the cause is unknown, not that it
/// costs nothing here.** It costs nothing *here* because
/// [`vibe_core::monitor::install`]'s `plan` and `state` take `home` as a
/// parameter, so every control plants its own and none of them touch this
/// function. That is the `list_prompts(.., user_home, ..)` precedent and it is
/// why the limit is survivable.
///
/// But anyone trying to drive the **CLI** against a fixture home will hit it,
/// and the failure looks exactly like holding it wrong: a correct-looking
/// `USERPROFILE`, a directory that is really there, and a flat refusal. Nothing
/// below diagnoses it, because nothing below knows why it happens.
///
/// **What was NOT established:** whether `BaseDirs::new()` is consulting
/// `USERPROFILE` at all on this platform, whether it validates the value
/// against something, or whether the override interacts with the known-folder
/// API. Reading the crate would produce a plausible answer, and a plausible
/// answer to *why did the instrument return nothing* is the shape this
/// repository catalogues. **The way to close it is to measure `BaseDirs::new()`
/// directly under each candidate environment**, which nobody has done.
fn resolve_ambient() -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let home = vibe_core::prompts::user_home().ok_or_else(|| {
        "this platform will not say where the home directory is, so the \
                        settings file cannot be located. Nothing was written."
            .to_owned()
    })?;
    let sink = vibe_core::agents::default_store_path()
        .map(|p| p.with_file_name("monitor"))
        .ok_or_else(|| {
            "this platform will not say where the data directory is, so the \
                        sink path cannot be resolved. Nothing was written."
                .to_owned()
        })?;
    // **The executable this process was started as.** Baked into the config, so
    // it goes stale when the binary moves — which is the staleness ADR-0011 §7b
    // accepted and which `vibe monitor status` is what makes visible.
    let command = std::env::current_exe().map_err(|e| {
        format!(
            "could not determine this executable's path ({e}), so the hook \
                              would have nothing to spawn. Nothing was written."
        )
    })?;
    Ok((home, command, sink))
}

/// `vibe monitor install`.
pub fn install_cmd(args: &MonitorInstallArgs) -> Result<Exit, CoreError> {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let json = args.format.json;

    // **Refused loudly, before anything is read or written.** The hook
    // validates the identity too, but there the message lands in a child
    // process's stderr that nobody reads. This is the half that can be made
    // loud, and it is the reason the flag exists rather than the identity being
    // taken from somewhere ambient.
    let identity = match WriterIdentity::parse(&args.identity) {
        Ok(i) => i,
        Err(rejection) => {
            let _ = writeln!(
                stderr,
                "vibe monitor install: the identity {:?} is not usable as a path \
                 component ({}). Nothing was written.",
                args.identity,
                rejection.key()
            );
            return Ok(Exit::Failure);
        }
    };

    let (home, command, sink) = match resolve_ambient() {
        Ok(t) => t,
        Err(why) => {
            let _ = writeln!(stderr, "vibe monitor install: {why}");
            return Ok(Exit::Failure);
        }
    };

    let planned = install::plan(&InstallRequest {
        home,
        command,
        sink,
        identity,
    })?;

    if json {
        let payload = serde_json::json!({
            "plan": planned.plan,
            "outcome": planned.outcome,
            "target": planned.target_path.display().to_string(),
        });
        let _ = writeln!(stdout, "{}", pretty(&payload));
    } else if planned.plan.is_empty() {
        // **A state, not a no-op to hide** (ADR-0011 §7b). Re-install is the
        // normal path, and a user who runs it twice is owed the difference
        // between "nothing needed doing" and "it worked".
        let _ = writeln!(
            stdout,
            "Already installed and up to date: {}",
            planned.target_path.display()
        );
        return Ok(Exit::Success);
    } else {
        let _ = output::write_plan_human(&mut stdout, &planned.plan);
    }

    if args.write.dry_run {
        if !json {
            let _ = writeln!(stderr, "\ndry run - nothing was written");
        }
        return Ok(Exit::Success);
    }
    if planned.plan.is_empty() {
        return Ok(Exit::Success);
    }

    let rep = reporter::TermReporter::new(json);
    let report = vibe_core::plan::apply(&planned.plan, &rep)?;
    if !json {
        let _ = output::write_apply_human(&mut stdout, &report);
        let verb = match planned.outcome {
            InstallOutcome::Installed { .. } => "installed",
            InstallOutcome::Upgraded { .. } => "upgraded",
            _ => "wrote",
        };
        let _ = writeln!(
            stdout,
            "\nHook {verb}. Restart any running Claude Code session: the config \
             is read at session start."
        );
    }
    rep.flush();
    Ok(Exit::Success)
}

/// `vibe monitor status`.
pub fn status_cmd(args: &MonitorStatusArgs) -> Result<Exit, CoreError> {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

    let identity = match WriterIdentity::parse(&args.identity) {
        Ok(i) => i,
        Err(rejection) => {
            let _ = writeln!(
                stderr,
                "vibe monitor status: the identity {:?} is not usable as a path \
                 component ({}).",
                args.identity,
                rejection.key()
            );
            return Ok(Exit::Failure);
        }
    };

    let home = match vibe_core::prompts::user_home() {
        Some(h) => h,
        None => {
            let _ = writeln!(
                stderr,
                "vibe monitor status: this platform will not say where the home \
                 directory is, so the settings file cannot be located."
            );
            return Ok(Exit::Failure);
        }
    };

    let state = install::state(&home, &identity)?;

    if args.format.json {
        let _ = writeln!(
            stdout,
            "{}",
            pretty(&serde_json::json!({ "install": state }))
        );
        return Ok(Exit::Success);
    }

    // **Each state says what silence would mean under it.** That is the whole
    // point of there being three: ADR-0011 §6's *absence of events is not a
    // state*, made legible at the place a user reads it.
    match &state {
        InstallState::NotInstalled => {
            let _ = writeln!(
                stdout,
                "not installed - no hook group declares {:?}.\n  \
                 No events can arrive, so silence says nothing about any agent.\n  \
                 Run `vibe monitor install` to wire it.",
                args.identity
            );
        }
        InstallState::Degraded {
            command,
            command_present,
            sink,
            sink_present,
        } => {
            let _ = writeln!(
                stdout,
                "installed, but something it names is gone.\n  \
                 command {} - {}\n  \
                 sink    {} - {}\n  \
                 Events have stopped arriving for a reason that has nothing to do \
                 with any agent, so silence must NOT be read as an idle one.",
                command.display(),
                if *command_present {
                    "present"
                } else {
                    "MISSING"
                },
                sink.display(),
                if *sink_present { "present" } else { "MISSING" },
            );
            if !command_present {
                let _ = writeln!(
                    stdout,
                    "  The binary moved. Re-run `vibe monitor install` to re-bake the path."
                );
            }
        }
        // **The label says what was checked, not how things are going.**
        // "installed and healthy" was the first wording and it is one word from
        // claiming delivery: `healthy` is a word about a running system, and
        // nothing here looked at the sink. What was checked is that two paths
        // exist. That is a statement about CAPABILITY, and §6 is the reason the
        // difference matters — absence of events is not a state, so a label
        // that lets a reader infer events are flowing has resolved the
        // ambiguity in the reassuring direction on no evidence.
        //
        // The variant is still called `Healthy` because it is the health of the
        // INSTALL, which is all this read can see. The sentence a user reads
        // does not inherit that word.
        InstallState::Healthy { command, sink } => {
            let _ = writeln!(
                stdout,
                "installed; both paths it names exist.\n  command {}\n  sink    {}\n  \
                 So events CAN arrive. This says nothing about whether any have: \
                 nothing here read the sink, and even a sink that was read cannot \
                 tell a quiet agent from one that stopped.\n  \
                 Checked: the config, not the events.",
                command.display(),
                sink.display()
            );
        }
        other => {
            let _ = writeln!(
                stdout,
                "this build cannot describe the install state it found ({}).",
                other.key()
            );
        }
    }
    Ok(Exit::Success)
}
