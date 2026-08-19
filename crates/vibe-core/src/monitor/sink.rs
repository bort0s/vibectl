//! The minimum read the writer's controls need — and no more.
//!
//! ADR-0011 §8 leaves the display shape open and §6 constrains it without
//! settling it, so nothing here renders. What is built is exactly what
//! §9's control (b) asserts on: the **tail state** of a file. Control (f)'s
//! subject — prunability — was **retracted**, see below.
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
//! # RETRACTED: prunability is not derivable from event content
//!
//! *2026-08-19.* This module used to derive a `Prunability` from *"the file
//! contains `SessionEnd`"*, on the ground that ADR-0011 §4 measured a killed
//! agent writing neither `Stop` nor `SessionEnd`.
//!
//! **`SessionEnd` is not terminal.** Measured: a resumed session emits
//! `SessionStart` *after* `SessionEnd` under one `session_id`, and a session two
//! hours old was resumed from a different working directory. The proposed
//! repair — a third state, *ended at least once and reopenable* — was checked
//! before being built and does not survive the check either: **`reopenable` is
//! always true**, so a variant whose predicate never varies distinguishes
//! nothing.
//!
//! So whether a file will receive more records is **not a function of the events
//! in it**, and constraint 5 says the field stays empty and flagged rather than
//! carrying a plausible value. The type is gone rather than weakened.
//!
//! # WHAT MUST NOT BE WRITTEN HERE: file-age prunability
//!
//! The next reader will propose *"a file untouched for N days is prunable"*, and
//! the reason it is refused needs to be here to meet them.
//!
//! It is the same claim with a worse basis. Age measures **when vibe last
//! received an event**, which is exactly the observable §7 spends its length
//! establishing means nothing on its own: a quiet agent, a removed hook and a
//! finished session all produce it. A file untouched for a month belongs to a
//! session somebody can still resume, and the tool cannot tell that from one
//! nobody will. Offering it under a label the user reads as *safe to delete* is
//! constraint 5's invented plausible value, pointed at their records.
//!
//! **What could ground it is an explicit user action** — they say this session
//! is done — because that is a fact vibe was told rather than one it inferred.
//! Nothing here is built for it, and §8 has not asked.
//!
//! **And vibe never deletes.** ADR-0001 §3 enforces constraint 2's
//! deletion-as-an-operation by the absence of `FileOp::Delete`, and an absent
//! enum variant has no scope, so it covers a sink vibe created just as it covers
//! a user's files. That is why the cost of the wrong label was bounded — but the
//! reason for the retraction is constraint 5, not the deletion risk, and that
//! reason survives §8 shipping a display where *"nothing renders it"* would not.

use std::path::{Path, PathBuf};

use serde::Serialize;

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
