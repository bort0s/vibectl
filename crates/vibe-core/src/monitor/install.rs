//! `vibe monitor install` — the plan it builds, and the state a later read finds.
//!
//! ADR-0011 §7's **one explicit act**. This module builds a [`WritePlan`] and
//! never writes: the only thing that touches the filesystem is
//! [`crate::plan::apply`], so `--dry-run` is the caller declining to make the
//! second call, exactly as it is everywhere else (ADR-0001 §3).
//!
//! # What install actually writes
//!
//! One [`FileOp::UpdateSettings`], through the closed route in
//! [`super::target`]. The editor in [`super::settings`] produces the text; this
//! module is what carries it to a plan and what reads it back afterwards.
//!
//! # Three states, not two
//!
//! The read side exists because *"no events"* is not one fact. §7's hazard is
//! that a file which stops growing is indistinguishable from a quiet agent, and
//! the config is where the two can be separated **before** the sink is
//! consulted at all:
//!
//! - **[`InstallState::NotInstalled`]** — no group declares this identity. No
//!   events is exactly what should be expected, and reporting *idle* here would
//!   be reporting on a hook that was never wired.
//! - **[`InstallState::Degraded`]** — a group is there, and something it names
//!   is gone. The baked command path or the baked sink root, or both. Events
//!   stopped arriving for a reason that has nothing to do with any agent.
//! - **[`InstallState::Healthy`]** — a group is there and everything it names
//!   exists. **Only here does silence say anything about an agent**, and even
//!   then only what §5 and §6 permit.
//!
//! The middle state is the one that must exist. Collapsing it into either
//! neighbour is §6's *absence of events is not a state* arriving through the
//! config rather than through the sink: an install whose binary was moved by a
//! `cargo install` reads as *not installed* (wrong — a stale group is still in
//! the user's file) or as *healthy and quiet* (wrong, and the dangerous
//! direction, because it invites reading silence as an agent finishing).
//!
//! # The staleness is inherited, not introduced
//!
//! §7b bakes the executable path and the sink root into the config at install
//! time, and both can go stale. That is a cost of the baking decision rather
//! than of this module — what this module owes is that the staleness is
//! **visible**, which is what [`InstallState::Degraded`] is for.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::CoreError;
use crate::plan::{FileOp, PlanIntent, WritePlan};

use super::identity::WriterIdentity;
use super::settings::{
    HookSpec, INSTALLED_EVENTS, InstallOutcome, SettingsDocument, declared_hook, install as edit,
    read_document,
};
use super::target::SettingsTarget;

/// Everything install needs, with nothing resolved from the environment here.
///
/// **`home` and `command` are passed in rather than resolved in this module**,
/// which is [`crate::prompts::list_prompts`]'s precedent and exists for the same
/// reason: a resolver buried in here makes every control below depend on the
/// real user's configuration and the real binary's location. The caller — the
/// CLI — resolves both in vibe's own process, which is also where §7a requires
/// the sink path to be resolved.
#[derive(Debug, Clone)]
pub struct InstallRequest {
    /// The home directory `<home>/.claude/settings.json` hangs off.
    pub home: PathBuf,
    /// The vibe executable the hook will spawn, resolved by the caller.
    pub command: PathBuf,
    /// The sink root, resolved by the caller and **declared** in argv rather
    /// than resolved by the hook (ADR-0011 §7a).
    pub sink: PathBuf,
    /// The identity this install declares.
    pub identity: WriterIdentity,
}

/// A plan, and what applying it would do.
///
/// The outcome is carried beside the plan rather than derived from it, because
/// *"nothing needed doing"* and *"it worked"* are different things a user is
/// owed and an empty op list cannot distinguish them from a refusal.
#[derive(Debug, Clone)]
pub struct PlannedInstall {
    pub plan: WritePlan,
    pub outcome: InstallOutcome,
    /// Where the write would land. Resolved here so a caller can name it
    /// without re-deriving it, and so the dry run and the write agree.
    pub target_path: PathBuf,
}

/// Build the plan for one install.
///
/// # Errors
///
/// [`CoreError::SettingsRefused`] when the file is not something vibe can
/// safely edit — every [`SettingsRefusal`] variant — and [`CoreError::Io`] when
/// it exists and cannot be read. **Nothing is written on either path**, because
/// nothing here writes at all.
pub fn plan(req: &InstallRequest) -> Result<PlannedInstall, CoreError> {
    let target = SettingsTarget::User;
    let path = target.resolve(&req.home);

    let text = read_document(&path).map_err(|source| CoreError::Io {
        path: path.clone(),
        source,
    })?;

    // **Absent is not an empty file, and the difference reaches the diff.** A
    // missing `settings.json` is the ordinary first install and renders as a
    // create with nothing on the before side; an empty one is a file the user
    // has, and `before: Some("")` is what makes `apply` refuse if it stops
    // being empty between planning and applying.
    let mut doc = match &text {
        Some(t) => SettingsDocument::parse(t).map_err(|refusal| CoreError::SettingsRefused {
            path: path.clone(),
            refusal,
        })?,
        None => SettingsDocument::empty(),
    };

    let spec = HookSpec {
        command: req.command.clone(),
        sink: req.sink.clone(),
        identity: req.identity.clone(),
    };

    let outcome = edit(&mut doc, &spec).map_err(|refusal| CoreError::SettingsRefused {
        path: path.clone(),
        refusal,
    })?;

    // **`Unchanged` produces an empty plan, not a plan that rewrites the file
    // with identical bytes.** Re-install is the normal path (§7b), and a plan
    // whose only op replaces a file with itself would still fail on a
    // `PlanStale` race, still touch the mtime, and still render a diff with
    // nothing in it. The state is reported instead.
    let ops = if matches!(outcome, InstallOutcome::Unchanged) {
        Vec::new()
    } else {
        vec![FileOp::UpdateSettings {
            target,
            before: text,
            after: doc.render(),
        }]
    };

    Ok(PlannedInstall {
        plan: WritePlan::new(
            PlanIntent::MonitorInstall,
            target.containment_root(&req.home),
            ops,
        ),
        outcome,
        target_path: path,
    })
}

/// What a later read finds where install wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum InstallState {
    /// No group declares this identity.
    ///
    /// **Silence here is fully explained by this fact.** Nothing is wired, so
    /// nothing can arrive, and no reading about any agent is available.
    NotInstalled,
    /// A group is there and something it names is gone.
    ///
    /// The fields are separate booleans rather than one "broken" flag because
    /// they fail for different reasons and need different repairs: a moved
    /// binary is a re-install, a missing sink root is usually a wiped data
    /// directory. One flag would make a user guess which.
    Degraded {
        #[serde(serialize_with = "crate::plan::lossy_path")]
        command: PathBuf,
        command_present: bool,
        #[serde(serialize_with = "crate::plan::lossy_path")]
        sink: PathBuf,
        sink_present: bool,
    },
    /// A group is there and everything it names exists.
    ///
    /// **This is the only state in which silence says anything about an
    /// agent**, and even here it says only what §5 and §6 permit — a file that
    /// stops growing is still indistinguishable from a quiet agent, and this
    /// state does not close that.
    Healthy {
        #[serde(serialize_with = "crate::plan::lossy_path")]
        command: PathBuf,
        #[serde(serialize_with = "crate::plan::lossy_path")]
        sink: PathBuf,
    },
}

impl InstallState {
    /// Stable key.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            InstallState::NotInstalled => "not_installed",
            InstallState::Degraded { .. } => "degraded",
            InstallState::Healthy { .. } => "healthy",
            // No catch-all arm, deliberately. `#[non_exhaustive]` binds
            // downstream crates and not this one, so an exhaustive match here
            // is a COMPILE ERROR when a fourth state is added - which is what
            // forces the author to decide what it means rather than letting it
            // fall into a bucket. A `_` arm would compile forever and quietly
            // label the new state "unrecognised" at exactly the moment somebody
            // needed to know what it was.
        }
    }
}

/// Read the state of an install.
///
/// **This is the read side and it takes no route** (ADR-0011 §7b): reads are
/// not `FileOp`s and need nothing from the write path.
///
/// # Errors
///
/// [`CoreError::SettingsRefused`] for a config vibe cannot parse or cannot
/// unambiguously own, and [`CoreError::Io`] for a file that exists and cannot
/// be read. **A malformed config is not `NotInstalled`** — that would report
/// *nothing is wired* about a file nothing was learned from, which is §3b's
/// defect at the config layer.
pub fn state(home: &Path, identity: &WriterIdentity) -> Result<InstallState, CoreError> {
    let target = SettingsTarget::User;
    let path = target.resolve(home);

    let Some(text) = read_document(&path).map_err(|source| CoreError::Io {
        path: path.clone(),
        source,
    })?
    else {
        return Ok(InstallState::NotInstalled);
    };

    let doc = SettingsDocument::parse(&text).map_err(|refusal| CoreError::SettingsRefused {
        path: path.clone(),
        refusal,
    })?;

    // **Every installed event is consulted, not just the first.** A config with
    // vibe's group on `SessionStart` alone is not an install this build made,
    // and `locate` is also where the uniqueness fault (§7a) surfaces — checking
    // one event would skip the check on the other four.
    let mut found: Option<&serde_json::Value> = None;
    for event in INSTALLED_EVENTS {
        let hook =
            declared_hook(&doc, event, identity).map_err(|refusal| CoreError::SettingsRefused {
                path: path.clone(),
                refusal,
            })?;
        if let Some(hook) = hook {
            found = Some(hook);
        }
    }

    let Some(hook) = found else {
        return Ok(InstallState::NotInstalled);
    };

    let command = hook
        .get("command")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_default();
    let sink = argv_value(hook, "--sink")
        .map(PathBuf::from)
        .unwrap_or_default();

    let command_present = command.exists();
    let sink_present = sink.exists();

    if command_present && sink_present {
        Ok(InstallState::Healthy { command, sink })
    } else {
        Ok(InstallState::Degraded {
            command,
            command_present,
            sink,
            sink_present,
        })
    }
}

/// The value following a flag in a hook's `args`.
fn argv_value(hook: &serde_json::Value, flag: &str) -> Option<String> {
    let args = hook.get("args")?.as_array()?;
    let strings: Vec<&str> = args.iter().filter_map(serde_json::Value::as_str).collect();
    let mut it = strings.iter();
    while let Some(a) = it.next() {
        if *a == flag {
            return it.next().map(|s| (*s).to_owned());
        }
    }
    None
}
