//! The minimum read the writer's controls need — and no more.
//!
//! ADR-0011 §8 leaves the display shape open and §6 constrains it without
//! settling it, so nothing here renders. What is built is exactly what
//! §9's controls (b) and (f) assert on: the **tail state** of a file, and
//! whether a file is **prunable**.
//!
//! # The tail check is positional, which is what makes it certain
//!
//! One writer per file means interleaving is impossible and only truncation
//! remains, so **only the last record can be partial**. The reader therefore
//! validates a *tail* rather than scanning for corruption anywhere.
//!
//! The frame is the trailing newline, and that choice is load-bearing. §9 (b)
//! requires the reader to report a partial tail *"not dropping it silently, and
//! not parsing a prefix that happens to be valid"* — so the test cannot be
//! *"does the last line parse"*. A truncated record can be valid JSON by
//! coincidence; it can never have the newline the writer appends last.
//!
//! # Prunability is derived from the artifact, never from remembered state
//!
//! ADR-0011 §7a: the reader is **stateless**, walking the sink and reading
//! everything, every time. A read-watermark would be a second artifact kept
//! equal to the files — the shape ADR-0010 §3 rejected — and its staleness is
//! silent in the dangerous direction, because a watermark ahead of the truth
//! *hides records*.
//!
//! So *"unconsumed"* is not a state vibe holds. What it can derive is that **a
//! file containing `SessionEnd` is a completed session**, and ADR-0011 §4
//! measured that a killed agent writes neither `Stop` nor `SessionEnd` — so a
//! file lacking it is in-progress-or-dead and must never be offered as
//! prunable. That rests on a measurement rather than on anyone's memory.
//!
//! **And vibe never deletes.** ADR-0001 §3 enforces constraint 2 by the absence
//! of `FileOp::Delete` — *a destructive command is not merely discouraged, it
//! is unrepresentable* — and an absent enum variant has no scope, so it covers
//! a sink vibe created just as it covers a user's files. This module computes
//! what is prunable and removes nothing.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The event name that marks a completed session (ADR-0011 §4).
pub const SESSION_END_EVENT: &str = "SessionEnd";

/// Whether the last record in a file is whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "tail", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TailState {
    /// The file ends on a record boundary. An empty file is complete: it has no
    /// last record to be partial.
    Complete,
    /// The file ends mid-record. Under one-writer-per-file this is truncation —
    /// a crashed hook, a full disk, a killed process — and it is the only
    /// corruption the transport admits.
    Partial { bytes: usize },
}

/// One line of a file, as read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "record", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReadRecord {
    /// A complete line that parsed.
    ///
    /// `session` and `event` are the writer's **hoist** — what it read out of
    /// the payload at write time. `payload` is the payload itself, verbatim.
    /// ADR-0011 §7a: the payload is the source of truth and the hoist is a
    /// reading, so anything that must be right reads the payload.
    Parsed {
        session: Option<String>,
        event: Option<String>,
        stamp_ns: Option<String>,
        /// The hook payload as it arrived, still a JSON string. Parsing it is
        /// the second parse §7a costed; only a caller that needs payload fields
        /// pays it.
        payload: Option<String>,
    },
    /// A complete line that did not parse. Distinct from a partial tail: this
    /// one **has its newline**, so it was written whole and is wrong for some
    /// other reason — a hand edit, a foreign writer. Reported, never dropped.
    Unparseable { bytes: usize },
}

/// A file in the sink, read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SinkFile {
    pub path: PathBuf,
    pub records: Vec<ReadRecord>,
    pub tail: TailState,
}

/// The result of trying to read one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "read", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SinkRead {
    Read(Box<SinkFile>),
    /// The file could not be read at all. Not an empty file, and not a file
    /// with no records — a fact vibe does not have, named as missing.
    Unreadable {
        path: PathBuf,
        kind: &'static str,
    },
}

/// Whether a file may be offered to the user as prunable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "prunability", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Prunability {
    /// The session completed: a `SessionEnd` record is present and the file
    /// ends on a record boundary.
    Prunable,
    /// Not offered. The reason is inside the variant, because *"still running"*
    /// and *"we do not understand the end of this file"* are different facts.
    NotPrunable { reason: NotPrunableReason },
}

/// Why a file is not offered as prunable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NotPrunableReason {
    /// No `SessionEnd`. In progress, or a killed agent — ADR-0011 §4 measured
    /// that those are indistinguishable here, and neither is prunable.
    NoSessionEnd,
    /// The tail is torn, so what the file contains past the last whole record
    /// is unknown.
    PartialTail,
}

impl SinkFile {
    /// Derive prunability from the artifact.
    #[must_use]
    pub fn prunability(&self) -> Prunability {
        if self.tail != TailState::Complete {
            return Prunability::NotPrunable {
                reason: NotPrunableReason::PartialTail,
            };
        }
        let ended = self.records.iter().any(|r| {
            matches!(
                r,
                ReadRecord::Parsed { event: Some(e), .. } if e == SESSION_END_EVENT
            )
        });
        if ended {
            Prunability::Prunable
        } else {
            Prunability::NotPrunable {
                reason: NotPrunableReason::NoSessionEnd,
            }
        }
    }
}

/// Read one file from the sink.
#[must_use]
pub fn read_file(path: &Path) -> SinkRead {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return SinkRead::Unreadable {
                path: path.to_path_buf(),
                kind: match e.kind() {
                    std::io::ErrorKind::NotFound => "not_found",
                    std::io::ErrorKind::PermissionDenied => "permission_denied",
                    _ => "other",
                },
            };
        }
    };

    // Lossy, deliberately. A record containing invalid UTF-8 is a fact about
    // the file, and refusing to read the whole file because of one bad byte
    // would lose every good record beside it. The bad line still fails to parse
    // and is reported as `Unparseable`.
    let text = String::from_utf8_lossy(&bytes);

    // The frame. Everything before the final newline is a whole record;
    // anything after it is a torn tail. An empty file has neither.
    let (whole, trailing) = match text.rfind('\n') {
        Some(i) => (&text[..=i], &text[i + 1..]),
        None => ("", text.as_ref()),
    };

    let tail = if trailing.is_empty() {
        TailState::Complete
    } else {
        TailState::Partial {
            bytes: trailing.len(),
        }
    };

    let records = whole
        .lines()
        .filter(|l| !l.is_empty())
        .map(parse_line)
        .collect();

    SinkRead::Read(Box::new(SinkFile {
        path: path.to_path_buf(),
        records,
        tail,
    }))
}

fn parse_line(line: &str) -> ReadRecord {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return ReadRecord::Unparseable { bytes: line.len() };
    };
    let Some(object) = value.as_object() else {
        return ReadRecord::Unparseable { bytes: line.len() };
    };
    let get = |k: &str| {
        object
            .get(k)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    ReadRecord::Parsed {
        session: get("session"),
        event: get("event"),
        stamp_ns: get("stamp_ns"),
        payload: get("payload"),
    }
}
