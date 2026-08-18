//! Appending one hook record to one file.
//!
//! ADR-0011 §7a decided the transport: **a file, appended to by a `command`
//! hook in the `args` exec form**, one file per `(session, declared writer
//! identity)`, *so concurrent append does not exist*.
//!
//! That last clause is the whole design and it is worth restating as a
//! constraint on this module rather than as background. Two shared-file shapes
//! were rejected: one relying on append atomicity has a failure that **cannot
//! be induced on demand**, so it cannot have a paired control and is
//! inadmissible before cost is discussed; one with framed records is
//! admissible but **detects rather than prevents**. What decided it is that
//! both rest on an atomicity guarantee this project **cannot measure on two of
//! its three platforms** — the same shape as the `PIPE_BUF` assumption that
//! died on contact with measurement. One writer per file does not handle that
//! dependency, it removes it.
//!
//! **So there is no locking here, and its absence is the design.** Adding one
//! would reintroduce exactly the cross-platform guarantee this shape was chosen
//! to stop depending on.
//!
//! # The positional bound, which is what the reader gets for free
//!
//! With one writer, interleaving is impossible and only truncation remains — a
//! crashed hook, a full disk, a killed process — so **only the last record in a
//! file can be partial**. The frame is the trailing newline, which makes that
//! *certainly* detectable rather than heuristically so: a file not ending in
//! `\n` has a torn tail, whether or not the bytes before it happen to parse.
//!
//! # Failing is not optional, and neither is being loud about it
//!
//! A hook that cannot write is **silent non-delivery arriving from our side** —
//! ADR-0011 §7's central hazard, produced by the mechanism installed to prevent
//! it. So this module may not swallow, and it may not panic either: a panic in
//! a hook is a nonzero exit the agent sees at best, and a process the agent is
//! waiting on dying at worst.
//!
//! [`Writer::append`] therefore returns [`WriteOutcome`] and **not a
//! `Result`** — the same technique [`crate::check_ignore`] uses for the same
//! reason: there is no `Result` for a `?` to collapse and no error type for a
//! caller to discard with `.ok()`. Every failure is a value the caller has to
//! match on.
//!
//! # The hook process exits `1` on a failed write, never `2`
//!
//! *Decided 2026-08-18 (ADR-0011 §7a). Stated here rather than only in the ADR,
//! because here is where the hook's `main` will be written from, and ADR-0002
//! §7 records what a correction left at the decision site and not at the copy
//! site cost: the disproved claim was reproduced twice more by an author who
//! read the neighbouring comment.*
//!
//! Not swallowing settles loudness to **us**. It does not settle loudness to the
//! **agent**, which is a separate question with a separate answer.
//!
//! Exit `2` is visible to the agent and can interrupt the turn. **The monitor is
//! additive — vibe works without it** — so a hook that stops the user's work
//! over its own write failure has inverted the relationship between observer and
//! observed, and **an observer that can stop the subject is not one.**
//!
//! The loss is not silent under `1`: [`WriteOutcome::Failed`] carries the stage,
//! the `ErrorKind`, the raw OS code and the torn-byte count, so the next read
//! reports it. Immediacy is the only thing surrendered, and it is surrendered in
//! the direction where the cost would otherwise land on someone who did not ask
//! for a monitor.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use super::identity::{ComponentRejection, SessionComponent, WriterIdentity};
use super::record::{CONTRACT_VERSION, Record, StampSource};

/// An I/O failure, kept at the granularity the operating system reports it.
///
/// **The raw OS code is carried, not collapsed.** ADR-0011 §5 records the cost
/// of the opposite: `OpenProcess` returning NULL is two outcomes wearing one
/// observable, and code that branches on the null handle merges *"exists and
/// cannot be read"* with *"there is no such process"* — in the direction that
/// reads as reassurance. The same discipline applies to a write: *permission
/// denied* and *disk full* have opposite remedies, and a single `io_error` key
/// would make them one fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IoFailure {
    /// A stable key for the `std::io::ErrorKind`.
    pub kind: &'static str,
    /// The platform's own error number, when there is one.
    pub os_code: Option<i32>,
}

impl IoFailure {
    fn from_io(e: &std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        // `ErrorKind` is `#[non_exhaustive]`, so a wildcard is unavoidable
        // here. It degrades to a named `other` rather than borrowing a
        // recognised value — ADR-0001 §4's rule for a frontend meeting a
        // variant it does not know.
        //
        // **The arms are bounded by the MSRV, not by what the platform can
        // produce.** `InvalidFilename`, `StorageFull` and `ReadOnlyFilesystem`
        // are all `io_error_more`, still unstable on the pinned 1.85.0 — the
        // `MSRV (1.85.0)` job caught them, which is the third time that job has
        // reported a fact rather than been fussy. So a disk-full write is
        // reported as `other` **with its raw OS code intact**, which is the half
        // that actually distinguishes it. When the MSRV moves, these become
        // named arms and the `os_code` stops being the only discriminator.
        let kind = match e.kind() {
            K::NotFound => "not_found",
            K::PermissionDenied => "permission_denied",
            K::AlreadyExists => "already_exists",
            K::WriteZero => "write_zero",
            K::InvalidInput => "invalid_input",
            K::Interrupted => "interrupted",
            _ => "other",
        };
        Self {
            kind,
            os_code: e.raw_os_error(),
        }
    }
}

/// Which step of the write failed.
///
/// Separate from [`IoFailure`] because *"permission denied creating the sink"*
/// and *"permission denied appending to a file inside it"* are different
/// problems with different remedies, and the `ErrorKind` is identical.
///
/// # DECLARED GAP: two of these four have never been exercised
///
/// The variants look uniform from outside and their coverage is not. Written
/// here rather than left for someone to infer from a green suite, because a
/// list that reads as complete and is not is the failure this project keeps
/// paying for.
///
/// | variant | control | how it is reached |
/// | --- | --- | --- |
/// | [`CreateSink`](WriteStage::CreateSink) | yes, paired | a file planted where the sink directory must go |
/// | [`OpenFile`](WriteStage::OpenFile) | yes, paired | a directory planted at the record path |
/// | [`Append`](WriteStage::Append) | **none** | needs a full disk or a process killed mid-write |
/// | [`Flush`](WriteStage::Flush) | **none** | same |
///
/// **Neither gap is inducible from this machine deterministically.** A full
/// volume is not constructible in a `tempfile::tempdir`, and killing the writing
/// process mid-`write_all` is a race — ADR-0002 §7 rejects a control whose
/// firing depends on winning one, because it can stop proving anything without
/// ever failing, and it arrives as a green check. So there is no control rather
/// than a flaky one, and no synthesised value dressed as a measurement.
///
/// **What that costs, precisely.** `torn_bytes` is computed only on these two
/// arms, so **the torn-tail size this writer reports has never been observed**.
/// The reader's tail check is controlled independently against a hand-built
/// truncated file, which is the half that matters for detection; what is
/// unverified is our own report of how much we tore.
///
/// **What would close it:** a filesystem the test controls the size of — a
/// small loopback or VHD image mounted for the fixture — which is real
/// machinery and is not built for two arms. The trigger to revisit is a third
/// uncontrolled arm, or a defect traced to one of these two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WriteStage {
    /// Creating the sink directory. **Controlled**, paired.
    CreateSink,
    /// Opening the record file for append. **Controlled**, paired.
    OpenFile,
    /// Writing the record bytes. **No control** — see the type docs.
    Append,
    /// Flushing them to the operating system. **No control** — see the type
    /// docs.
    Flush,
}

/// Why a payload could not be turned into a record.
///
/// **A refusal loses the event, and that is stated rather than softened.** The
/// alternative is worse: a payload with no usable session id has no filename,
/// and inventing one — a literal `unknown`, a hash, a fallback bucket — is
/// exactly the value-invention this whole feature exists to refuse, with the
/// added hazard that the invented name can collide with a real session and
/// reproduce the twin writer.
///
/// The loss is announced through the caller's exit status and never through
/// silence. ADR-0011 §7 already handles the consequence: absence of events is
/// not a state, and a session with no wiring proof is *unknown*, never idle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "refusal", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PayloadRefusal {
    /// The payload is not JSON at all.
    NotJson,
    /// The payload is JSON but not an object.
    NotAnObject,
    /// No `session_id` member. ADR-0011 §3 measured that every payload carries
    /// one on 2.1.233 — and that is a property of a build, so its absence is
    /// handled rather than assumed away.
    NoSessionId,
    /// `session_id` is present and is not a string.
    SessionIdNotString,
    /// `session_id` is a string that is not usable as a path component.
    SessionRejected { rejection: ComponentRejection },
}

impl PayloadRefusal {
    /// Stable key, safe to branch on and to print as data.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            PayloadRefusal::NotJson => "not_json",
            PayloadRefusal::NotAnObject => "not_an_object",
            PayloadRefusal::NoSessionId => "no_session_id",
            PayloadRefusal::SessionIdNotString => "session_id_not_string",
            PayloadRefusal::SessionRejected { .. } => "session_rejected",
        }
    }
}

/// What happened to one record.
///
/// Three outcomes, and the third is not decoration: *written*, *refused because
/// the payload cannot name a file*, and *the filesystem would not take it*.
/// Merging the last two would report a full disk as a bad payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[non_exhaustive]
pub enum WriteOutcome {
    /// The record is on disk and flushed.
    Written {
        path: PathBuf,
        bytes: usize,
        /// Whether the record carries an authored stamp. `false` means the
        /// clock could not be read and the field was **omitted** rather than
        /// zeroed.
        stamped: bool,
    },
    /// The payload could not name a file. The event is lost and the caller must
    /// say so.
    Refused { reason: PayloadRefusal },
    /// The filesystem refused. The event is lost and the caller must say so.
    Failed {
        stage: WriteStage,
        path: PathBuf,
        io: IoFailure,
        /// Bytes that reached the file before the failure, measured by
        /// re-statting rather than assumed to be zero.
        ///
        /// **Nonzero means a torn tail was just created by us**, which is
        /// precisely the case the reader's tail check exists for. Reporting it
        /// turns *"the file is corrupt"* into *"we corrupted it, here, then"*.
        torn_bytes: u64,
    },
}

impl WriteOutcome {
    /// Stable key for the outcome.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            WriteOutcome::Written { .. } => "written",
            WriteOutcome::Refused { .. } => "refused",
            WriteOutcome::Failed { .. } => "failed",
        }
    }

    /// Whether the event reached the sink. `false` for both failure shapes.
    #[must_use]
    pub fn delivered(&self) -> bool {
        matches!(self, WriteOutcome::Written { .. })
    }
}

/// Appends records for one declared identity into one sink directory.
#[derive(Debug, Clone)]
pub struct Writer {
    sink: PathBuf,
    identity: WriterIdentity,
    stamps: Arc<dyn StampSource>,
}

impl Writer {
    /// Build a writer.
    ///
    /// The identity is already validated — it is a [`WriterIdentity`], which
    /// cannot be constructed from an unchecked string. Validation therefore
    /// happens **at write as well as at install**, per ADR-0011 §7a, which is
    /// required because §7 permits hand-installed hooks that install never
    /// sees.
    #[must_use]
    pub fn new(
        sink: impl Into<PathBuf>,
        identity: WriterIdentity,
        stamps: Arc<dyn StampSource>,
    ) -> Self {
        Self {
            sink: sink.into(),
            identity,
            stamps,
        }
    }

    /// The declared identity this writer writes under.
    #[must_use]
    pub fn identity(&self) -> &WriterIdentity {
        &self.identity
    }

    /// Where a record for one session lands.
    ///
    /// `<session>__<identity>.jsonl`. Both components are validated path
    /// components, so this cannot escape the sink and cannot name a device.
    #[must_use]
    pub fn record_path(&self, session: &SessionComponent) -> PathBuf {
        self.sink.join(format!(
            "{}__{}.jsonl",
            session.as_str(),
            self.identity.as_str()
        ))
    }

    /// Append one hook payload.
    ///
    /// Returns a value in every case. Nothing here panics, nothing here
    /// silently succeeds, and there is no `Result` to discard.
    pub fn append(&self, payload: &str) -> WriteOutcome {
        let session = match session_of(payload) {
            Ok(s) => s,
            Err(reason) => return WriteOutcome::Refused { reason },
        };
        let event = event_of(payload);
        let path = self.record_path(&session);

        let stamp = self.stamps.now();
        let record = Record {
            contract: CONTRACT_VERSION,
            identity: self.identity.as_str(),
            session: session.as_str(),
            event: event.as_deref(),
            stamp: stamp.map(|s| s.to_digits()),
            payload,
        };

        // Serialising a `Record` cannot fail: every field is a string, an
        // `Option<String>` or a `&str`, and none of them has a custom
        // `Serialize` that can error. `unwrap_or_default` would hide a future
        // field that can, so the failure is routed to the same loud place as
        // every other one.
        let Ok(mut line) = serde_json::to_string(&record) else {
            return WriteOutcome::Failed {
                stage: WriteStage::Append,
                path,
                io: IoFailure {
                    kind: "serialize_failed",
                    os_code: None,
                },
                torn_bytes: 0,
            };
        };
        line.push('\n');

        if let Err(e) = fs::create_dir_all(&self.sink) {
            return WriteOutcome::Failed {
                stage: WriteStage::CreateSink,
                path,
                io: IoFailure::from_io(&e),
                torn_bytes: 0,
            };
        }

        let mut file = match OpenOptions::new().append(true).create(true).open(&path) {
            Ok(f) => f,
            Err(e) => {
                return WriteOutcome::Failed {
                    stage: WriteStage::OpenFile,
                    path,
                    io: IoFailure::from_io(&e),
                    torn_bytes: 0,
                };
            }
        };

        let before = len_of(&path);

        if let Err(e) = file.write_all(line.as_bytes()) {
            return WriteOutcome::Failed {
                stage: WriteStage::Append,
                path: path.clone(),
                io: IoFailure::from_io(&e),
                torn_bytes: len_of(&path).saturating_sub(before),
            };
        }
        if let Err(e) = file.flush() {
            return WriteOutcome::Failed {
                stage: WriteStage::Flush,
                path: path.clone(),
                io: IoFailure::from_io(&e),
                torn_bytes: len_of(&path).saturating_sub(before),
            };
        }

        WriteOutcome::Written {
            bytes: line.len(),
            path,
            stamped: stamp.is_some(),
        }
    }
}

/// File length, or `0` when it cannot be read.
///
/// Only ever used to compute a torn-tail size on a path that has already
/// failed, where a second failure has nothing left to report through.
fn len_of(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Pull `session_id` out of a payload and validate it as a path component.
fn session_of(payload: &str) -> Result<SessionComponent, PayloadRefusal> {
    let value: serde_json::Value =
        serde_json::from_str(payload).map_err(|_| PayloadRefusal::NotJson)?;
    let object = value.as_object().ok_or(PayloadRefusal::NotAnObject)?;
    let raw = object
        .get("session_id")
        .ok_or(PayloadRefusal::NoSessionId)?
        .as_str()
        .ok_or(PayloadRefusal::SessionIdNotString)?;
    SessionComponent::parse(raw).map_err(|rejection| PayloadRefusal::SessionRejected { rejection })
}

/// Pull `hook_event_name` out of a payload, if it carries one.
///
/// Absence is `None` rather than a placeholder: an event with no name is a fact
/// about the payload, and naming it `unknown` would put a string in a field a
/// reader branches on.
fn event_of(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    value
        .as_object()?
        .get("hook_event_name")?
        .as_str()
        .map(str::to_owned)
}
