//! Editing `settings.json` — a file vibe does not own.
//!
//! ADR-0011 §7 permits **one explicit `vibe monitor install`**, never automatic
//! repair, and §7b decides what it writes and where. This module is the edit
//! itself: read the document, put vibe's hook group in it, and hand back text
//! for a [`FileOp::UpdateFile`](crate::FileOp) so `--dry-run` can show the diff
//! before anything lands (ADR-0001 §3).
//!
//! # What has to be preserved, measured rather than assumed
//!
//! **Not comments.** ADR-0011 §7 measured that this file is **strict JSON** on
//! 2.1.234: a `//` comment, a `/* */` comment and a trailing comma are each
//! rejected as *"Invalid or malformed JSON"*, paired against a strict file. So
//! the thing `toml_edit` exists to protect for manifests cannot be here, and
//! the far cheaper `serde_json` route is admissible where it was not for
//! `.vibe/project.toml` (ADR-0002 §5).
//!
//! **Key order**, which is why the workspace enables `serde_json/preserve_order`
//! — see `serde_json_in_this_build_preserves_key_order`, which fails if that
//! feature is ever dropped, because nothing else would.
//!
//! **Unknown keys**, for the same reason ADR-0002 §5 preserves them in a
//! manifest: this build's idea of the schema is a snapshot of somebody else's,
//! and a key it does not recognise is far more likely to be newer than wrong.
//!
//! **Indentation and the line ending**, by sniffing them off the file rather
//! than imposing this crate's taste. Measured on a 49-line settings file that
//! already contains a vibe hook: re-serialising with a fixed two-space indent
//! rewrites **44 of 49** lines if the file is 4-space, **46** if it is
//! tab-indented, and **48** if it is CRLF — and **0** in every case once the
//! existing style is reused.
//!
//! # The residuals are declared, because a silent heuristic is the hazard
//!
//! - **Nothing to sniff** — a minified file, or one with no indented line.
//!   Falls back to two spaces.
//! - **Mixed indentation, and mixed line endings.** Both sniffs take the first
//!   occurrence, so a file that disagrees with itself is normalised to whichever
//!   came first and the rest is rewritten. One heuristic applied twice, and both
//!   are named: declaring only one would read as if the other had been checked.
//!
//! Neither is a blocker, because both land in the diff `--dry-run` renders
//! before the write. That is what constraint 2 is for.
//!
//! # Idempotency is the axis, not existence
//!
//! **Re-install and upgrade are the normal path.** Refusing to write a file that
//! already exists would make `install` unusable for anyone who has configured
//! anything, and the case that actually matters is *the file already contains a
//! vibe hook*.
//!
//! So vibe's group is **found by the identity it declares**, which is the value
//! §7a already uses to tell writers apart, and then replaced whole. Anything
//! this module cannot recognise as its own is **reported, never repaired** —
//! §7's rule for a contract mismatch, applied to the config.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use serde_json::ser::PrettyFormatter;

use super::identity::WriterIdentity;
use super::record::CONTRACT_VERSION;

/// The events install declares (ADR-0011 §7b).
///
/// **The lifecycle five and nothing else.** Every one is in the measured ten
/// (§2). The activity-rate events — `PreToolUse`, `PostToolUse`,
/// `PostToolBatch`, `MessageDisplay`, `UserPromptSubmit` — are deferred because
/// nobody has measured write volume for a real session, and nothing from the
/// twenty-one that never fired is installed on a guess.
pub const INSTALLED_EVENTS: [&str; 5] = [
    "SessionStart",
    "SessionEnd",
    "SubagentStart",
    "SubagentStop",
    "Stop",
];

/// The `timeout` install writes, in seconds.
///
/// **Derived, not chosen** — and the derivation is a control rather than a
/// comment: see `the_installed_timeout_is_what_the_rule_derives`, which
/// recomputes it from the cold-start measurement on whichever platform it is
/// running on. A platform whose hook is slow enough to push the multiplier past
/// the floor turns that red instead of leaving this number stale.
///
/// **Provisional until three platforms have reported.** The rule names the
/// largest maximum across all three; only `win32-x64` has one so far (37.5 ms,
/// test profile).
pub const HOOK_TIMEOUT_SECS: u64 = 5;

/// The multiplier in the rule. **It has never bound**: 37.5 ms × 100 is 3.75 s,
/// under the floor, so the floor has decided every time. Recorded because a
/// two-branch rule with one branch never taken carries an untested claim — that
/// the multiplier does anything — and it starts binding the moment a platform
/// reports over 50 ms.
pub const HOOK_TIMEOUT_MULTIPLIER: u32 = 100;

/// The floor in the rule, in seconds.
pub const HOOK_TIMEOUT_FLOOR_SECS: u64 = 5;

/// A matcher that matches everything.
///
/// **Measured on the class install writes**, not on a tool event: `"*"` fires
/// on all five lifecycle events, paired against a group with no matcher (§2,
/// round 3d). The first measurement was on `PreToolUse`, where a matcher
/// filters tool names — a control proving one hazard class while shipping
/// against another.
pub const MATCH_ALL: &str = "*";

/// Why a settings document could not be read or edited.
///
/// **Reported, never repaired** (ADR-0011 §7). Every variant here is a fact
/// about a file vibe does not own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SettingsRefusal {
    /// The bytes are not strict JSON. Measured: Claude Code refuses the same
    /// file, so this is not vibe being stricter than the consumer.
    NotJson { detail: String },
    /// Valid JSON, but not an object at the top level.
    NotAnObject,
    /// `hooks` is present and is not an object.
    HooksNotAnObject,
    /// `hooks.<event>` is present and is not an array of matcher-groups.
    EventNotAnArray { event: String },
    /// A group under `hooks.<event>` is not an object.
    GroupNotAnObject { event: String, index: usize },
    /// A group's `hooks` member is present and is not an array.
    GroupHooksNotAnArray { event: String, index: usize },
    /// **The declared identity was found in a group vibe did not write.** The
    /// group holds other hooks beside it, so replacing the group would delete
    /// somebody else's configuration and replacing the hook alone would leave
    /// vibe's delivery subject to a `matcher` vibe never wrote (§7b).
    IdentitySharesAGroup { event: String, index: usize },
    /// The declared identity appears in more than one group for one event.
    /// §7a's uniqueness fault, seen at the place §7a puts the check.
    IdentityDeclaredTwice { event: String },
}

impl SettingsRefusal {
    /// Stable key.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            SettingsRefusal::NotJson { .. } => "not_json",
            SettingsRefusal::NotAnObject => "not_an_object",
            SettingsRefusal::HooksNotAnObject => "hooks_not_an_object",
            SettingsRefusal::EventNotAnArray { .. } => "event_not_an_array",
            SettingsRefusal::GroupNotAnObject { .. } => "group_not_an_object",
            SettingsRefusal::GroupHooksNotAnArray { .. } => "group_hooks_not_an_array",
            SettingsRefusal::IdentitySharesAGroup { .. } => "identity_shares_a_group",
            SettingsRefusal::IdentityDeclaredTwice { .. } => "identity_declared_twice",
        }
    }
}

/// A settings file, parsed, with the formatting it arrived in.
#[derive(Debug, Clone)]
pub struct SettingsDocument {
    value: Value,
    indent: String,
    newline: &'static str,
    trailing_newline: bool,
}

impl SettingsDocument {
    /// Parse, keeping the formatting.
    ///
    /// # Errors
    ///
    /// [`SettingsRefusal::NotJson`] or [`SettingsRefusal::NotAnObject`].
    pub fn parse(text: &str) -> Result<Self, SettingsRefusal> {
        let value: Value = serde_json::from_str(text).map_err(|e| SettingsRefusal::NotJson {
            detail: e.to_string(),
        })?;
        if !value.is_object() {
            return Err(SettingsRefusal::NotAnObject);
        }
        Ok(Self {
            indent: sniff_indent(text).to_owned(),
            newline: sniff_newline(text),
            trailing_newline: text.ends_with('\n'),
            value,
        })
    }

    /// An empty document, for the case where no settings file exists yet.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            value: Value::Object(serde_json::Map::new()),
            indent: DEFAULT_INDENT.to_owned(),
            newline: "\n",
            trailing_newline: true,
        }
    }

    /// The indent this document is written with.
    #[must_use]
    pub fn indent(&self) -> &str {
        &self.indent
    }

    /// The line ending this document is written with.
    #[must_use]
    pub fn newline(&self) -> &'static str {
        self.newline
    }

    /// Render back to text, in the formatting it arrived in.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = Vec::new();
        let fmt = PrettyFormatter::with_indent(self.indent.as_bytes());
        let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
        self.value
            .serialize(&mut ser)
            .expect("a parsed Value re-serialises");
        let mut text = String::from_utf8(out).expect("serde_json emits utf-8");
        if self.trailing_newline {
            text.push('\n');
        }
        if self.newline == "\r\n" {
            // A literal newline BYTE can only be formatting: JSON escapes a
            // newline inside a string as backslash-n, so a global replace
            // cannot reach into a payload. Asserted in the controls both ways.
            text = text.replace('\n', "\r\n");
        }
        text
    }
}

/// The default indent when there is nothing to sniff.
const DEFAULT_INDENT: &str = "  ";

/// The first indented line's leading whitespace, or the default.
fn sniff_indent(text: &str) -> &str {
    for line in text.split('\n') {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('\r') {
            continue;
        }
        let ws = &line[..line.len() - trimmed.len()];
        if !ws.is_empty() {
            return ws;
        }
    }
    DEFAULT_INDENT
}

/// The first line ending in the file.
fn sniff_newline(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// What install writes into the config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookSpec {
    /// The executable, resolved at install time in vibe's own process.
    pub command: PathBuf,
    /// The sink root, resolved at install time and **declared** in argv rather
    /// than resolved by the hook (ADR-0011 §7a).
    pub sink: PathBuf,
    /// The identity this hook declares. Half a filename, so it is validated as
    /// a path component before it gets here.
    pub identity: WriterIdentity,
}

impl HookSpec {
    /// The hook object, with every field install depends on written out.
    ///
    /// **Not verbosity — it deletes a dependency.** Each of these was measured
    /// (ADR-0011 §2, round 3b) and each omission would otherwise leave
    /// install's correctness resting on an upstream default with nothing
    /// watching it.
    ///
    /// **`shell` is deliberately absent**, and that is a correction rather than
    /// an omission: it is a string enum of `bash`/`powershell` with no value
    /// meaning *no shell*, and the loader refuses `false`. What keeps a shell
    /// out of the channel is `args` — the exec form — which is why
    /// `the_emitted_hook_uses_the_args_exec_form` exists.
    ///
    /// **`if` is deliberately absent too**, and it is the one residual
    /// omission-dependency: no value meaning *do not suppress* exists, and
    /// `if: "*"` is accepted by the loader and fires nothing.
    #[must_use]
    pub fn hook_object(&self) -> Value {
        let mut hook = serde_json::Map::new();
        hook.insert("type".into(), Value::String("command".into()));
        hook.insert(
            "command".into(),
            Value::String(self.command.to_string_lossy().into_owned()),
        );
        hook.insert(
            "args".into(),
            Value::Array(
                self.argv()
                    .into_iter()
                    .map(Value::String)
                    .collect::<Vec<_>>(),
            ),
        );
        hook.insert("once".into(), Value::Bool(false));
        hook.insert("async".into(), Value::Bool(false));
        hook.insert("asyncRewake".into(), Value::Bool(false));
        hook.insert("timeout".into(), Value::from(HOOK_TIMEOUT_SECS));
        Value::Object(hook)
    }

    /// The argv the hook is spawned with.
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        vec![
            "monitor".into(),
            "hook".into(),
            "--identity".into(),
            self.identity.as_str().to_owned(),
            "--sink".into(),
            self.sink.to_string_lossy().into_owned(),
            "--contract".into(),
            CONTRACT_VERSION.to_owned(),
        ]
    }

    /// The whole matcher-group, which install owns end to end.
    ///
    /// **Its own group, never appended into somebody else's.** `matcher` lives
    /// on the group, and a non-matching one suppresses every hook inside it
    /// (measured, §2 round 3b) — so joining an existing group would put vibe's
    /// delivery under a filter vibe never wrote.
    #[must_use]
    pub fn group(&self) -> Value {
        let mut group = serde_json::Map::new();
        group.insert("matcher".into(), Value::String(MATCH_ALL.into()));
        group.insert("hooks".into(), Value::Array(vec![self.hook_object()]));
        Value::Object(group)
    }
}

/// What an install did to a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[non_exhaustive]
pub enum InstallOutcome {
    /// No group declaring this identity was there; one was added.
    Installed { events: Vec<String> },
    /// A group declaring this identity was there and differed; it was replaced.
    Upgraded { events: Vec<String> },
    /// Every event already carries exactly this group. Nothing was written.
    ///
    /// **A state, not a no-op to hide.** Re-install is the normal path, and a
    /// user who runs it twice is owed the difference between *"nothing needed
    /// doing"* and *"it worked"*.
    Unchanged,
}

impl InstallOutcome {
    /// Stable key.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            InstallOutcome::Installed { .. } => "installed",
            InstallOutcome::Upgraded { .. } => "upgraded",
            InstallOutcome::Unchanged => "unchanged",
        }
    }
}

/// Put vibe's group into a document, replacing an earlier one.
///
/// # Errors
///
/// Every [`SettingsRefusal`] variant that is not a parse failure: a shape this
/// module does not recognise, or the identity found somewhere vibe cannot own.
/// **Nothing is written on a refusal** — the document is left as it was.
pub fn install(
    doc: &mut SettingsDocument,
    spec: &HookSpec,
) -> Result<InstallOutcome, SettingsRefusal> {
    // Validate the whole document before touching any of it, so a refusal on
    // the fifth event cannot leave the first four rewritten. `apply` is
    // all-or-nothing at the decision level (ADR-0001 §3) and this is the same
    // rule one layer in.
    let mut plan: Vec<(String, Option<usize>)> = Vec::new();
    for event in INSTALLED_EVENTS {
        plan.push((event.to_owned(), locate(doc, event, spec)?));
    }

    let desired = spec.group();
    let mut changed_events = Vec::new();

    let root = doc.value.as_object_mut().expect("checked at parse");
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .expect("checked by locate");

    let mut any_existing = false;
    for (event, at) in plan {
        let groups = hooks
            .entry(event.clone())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("checked by locate");
        match at {
            Some(i) => {
                any_existing = true;
                if groups[i] != desired {
                    groups[i] = desired.clone();
                    changed_events.push(event);
                }
            }
            None => {
                groups.push(desired.clone());
                changed_events.push(event);
            }
        }
    }

    if changed_events.is_empty() {
        Ok(InstallOutcome::Unchanged)
    } else if any_existing {
        Ok(InstallOutcome::Upgraded {
            events: changed_events,
        })
    } else {
        Ok(InstallOutcome::Installed {
            events: changed_events,
        })
    }
}

/// Where vibe's group sits under one event, if it is there at all.
///
/// **Found by the declared identity**, which is what §7a already uses to tell
/// writers apart — not by the command path, which changes when the binary
/// moves, and not by position, which changes when a user edits the file.
fn locate(
    doc: &SettingsDocument,
    event: &str,
    spec: &HookSpec,
) -> Result<Option<usize>, SettingsRefusal> {
    let Some(hooks) = doc.value.get("hooks") else {
        return Ok(None);
    };
    if !hooks.is_object() {
        return Err(SettingsRefusal::HooksNotAnObject);
    }
    let Some(groups) = hooks.get(event) else {
        return Ok(None);
    };
    let Some(groups) = groups.as_array() else {
        return Err(SettingsRefusal::EventNotAnArray {
            event: event.to_owned(),
        });
    };

    let mut found: Option<usize> = None;
    for (index, group) in groups.iter().enumerate() {
        let Some(group) = group.as_object() else {
            return Err(SettingsRefusal::GroupNotAnObject {
                event: event.to_owned(),
                index,
            });
        };
        let inner = match group.get("hooks") {
            None => continue,
            Some(v) => v.as_array().ok_or(SettingsRefusal::GroupHooksNotAnArray {
                event: event.to_owned(),
                index,
            })?,
        };
        let declaring = inner.iter().filter(|h| declares(h, spec)).count();
        if declaring == 0 {
            continue;
        }
        if inner.len() > declaring {
            // Vibe's hook is sitting beside somebody else's inside one group.
            // Replacing the group deletes their config; replacing the hook
            // leaves vibe under their `matcher`. Neither is ours to choose.
            return Err(SettingsRefusal::IdentitySharesAGroup {
                event: event.to_owned(),
                index,
            });
        }
        if found.is_some() {
            return Err(SettingsRefusal::IdentityDeclaredTwice {
                event: event.to_owned(),
            });
        }
        found = Some(index);
    }
    Ok(found)
}

/// Whether one hook object declares this install's identity.
fn declares(hook: &Value, spec: &HookSpec) -> bool {
    let Some(args) = hook.get("args").and_then(Value::as_array) else {
        return false;
    };
    let strings: Vec<&str> = args.iter().filter_map(Value::as_str).collect();
    let mut it = strings.iter();
    while let Some(a) = it.next() {
        if *a == "--identity" {
            return it.next().copied() == Some(spec.identity.as_str());
        }
    }
    false
}

/// Read a settings file that may not exist.
///
/// **Absent is not an error and not an empty file.** A missing
/// `settings.json` is the ordinary first install; an unreadable one is a fact
/// vibe must report rather than overwrite.
///
/// # Errors
///
/// The io error, when the file exists and cannot be read.
pub fn read_document(path: &Path) -> Result<Option<String>, std::io::Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
