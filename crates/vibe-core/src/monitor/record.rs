//! One line of the sink, and the authored stamp on it.
//!
//! # The stamp is a fallback, and saying so is most of its documentation
//!
//! ADR-0011 §7a measured that **no event type carries a timestamp**. Across all
//! eight observed types the payload offers `session_id`, `prompt_id`,
//! `turn_id`, `message_id`, `tool_use_id`, `index` and `duration_ms`, and
//! nothing naming a point in time. So under a per-writer-file transport, where
//! history is reassembled from several files, **the merge key is a value the
//! writer invents** — constraint 5 pointed at ordering rather than at a value,
//! and the more dangerous target, because a wrong value is a wrong field while
//! a wrong order is a *plausible history*.
//!
//! What the payload does support at no cost and with no clock is a **partial
//! order**: `session_id` groups, `prompt_id` groups a turn, `tool_use_id` pairs
//! a `PreToolUse` with its `PostToolUse`, `index` sequences `MessageDisplay`
//! deltas. **That is the primary ordering.** [`Stamp`] is the fallback, used
//! only where the payload orders nothing.
//!
//! # What the stamp claims
//!
//! *This record was written at this wall-clock instant, as read by this process
//! at the moment of writing.* That is all.
//!
//! # What it does not claim, itemised because the temptation is to assume each
//!
//! - **Not comparable across processes.** Two hook processes read their own
//!   clocks. ADR-0011 §7a records a boot-relative monotonic clock as
//!   *"deliberately not taken yet"* — it is **unmeasured on all three
//!   platforms**, and the last cross-platform property accepted on the standing
//!   of *"is understood to be"* was `PIPE_BUF`, which died on contact with
//!   measurement. This module does not build on that standing and does not
//!   pretend the measurement will come out well.
//! - **Not monotonic.** A wall clock steps, under NTP and under a user setting
//!   it. Two records can carry equal or inverted stamps with no defect present.
//! - **Not an ordering authority.** Where the payload orders two records, the
//!   payload wins. Where neither orders them, the reader's contract is to
//!   report them as *unordered*, never to present one sequence.
//!
//! # The one check the stamp does support
//!
//! Within a single file there is exactly one writer, so stamps must be
//! non-decreasing. **A decreasing stamp inside one file is direct evidence the
//! clock stepped** — a reported fact rather than an inference, detectable with
//! no cross-file reasoning at all. That check belongs to the reader and is not
//! built in this phase.
//!
//! # Why the stamp is a string
//!
//! Nanoseconds since the epoch is ~1.7 × 10^18, which is past 2^53. A JSON
//! number that large **loses precision in any reader backed by an IEEE double**,
//! which is every JavaScript one and several `jq` builds. Emitting decimal
//! digits in a string costs a parse and cannot silently round. A stamp that
//! quietly changes value on the way out would be an instrument altering its own
//! measurement.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// The contract version this writer implements.
///
/// ADR-0011 §7 requires the installed hook to declare which contract version it
/// implements so a mismatch can be **reported, never repaired** — repair would
/// be sync under another name, done at the moment the user is least able to see
/// it.
///
/// **It pins execution properties, not only payload shape.** Hooks carry
/// `timeout`, `async` and `asyncRewake`, and each changes *whether and when* a
/// record arrives: an `async` hook killed at session end delivers nothing, and
/// a `timeout` that fires truncates a record. A contract pinning only the
/// payload leaves the delivery semantics unpinned, which is the half that
/// decides whether absence means anything.
pub const CONTRACT_VERSION: &str = "1";

/// An authored wall-clock stamp, in nanoseconds since the Unix epoch, UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Stamp {
    nanos: u128,
}

impl Stamp {
    /// Construct from raw nanoseconds. For fixtures and for the reader.
    #[must_use]
    pub fn from_nanos(nanos: u128) -> Self {
        Self { nanos }
    }

    /// The raw nanoseconds.
    #[must_use]
    pub fn nanos(&self) -> u128 {
        self.nanos
    }

    /// Decimal digits, as written to the sink. See the module docs for why this
    /// is not a JSON number.
    #[must_use]
    pub fn to_digits(&self) -> String {
        self.nanos.to_string()
    }
}

/// Where a stamp comes from, injected rather than read ambiently.
///
/// `config.rs` opens with *"no global state, no ambient clock"* and gets a
/// [`crate::config::Clock`] for the same reason: a test that reads the system
/// clock asserts against a value that changes underneath it. That trait answers
/// *"what day is it"* to stamp a manifest; this one needs sub-second resolution
/// and a different failure mode, so it is a second trait rather than a widening
/// of the first.
pub trait StampSource: Send + Sync + std::fmt::Debug {
    /// The current instant, or the reason there is not one.
    fn now(&self) -> Option<Stamp>;
}

/// The system wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemStamps;

impl StampSource for SystemStamps {
    /// `None` when the clock reads **before the Unix epoch**, which
    /// `SystemTime::duration_since` reports as an error.
    ///
    /// Returned as absence rather than coerced to `0`, because a zero stamp is
    /// a plausible value in the right shape — the failure class ADR-0002 §7
    /// records for .NET's `Process.StartTime`, where a wrapper degraded to a
    /// well-formed empty value and invited a whole design paragraph about an
    /// arm that did not exist.
    fn now(&self) -> Option<Stamp> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| Stamp::from_nanos(d.as_nanos()))
    }
}

/// A fixed sequence of stamps. For fixtures.
///
/// Yields each value once, in order, then `None` — so a test that writes more
/// records than it planted stamps for gets the *absent* arm rather than a
/// silently repeated value.
#[derive(Debug)]
pub struct FixedStamps {
    remaining: std::sync::Mutex<std::vec::IntoIter<u128>>,
}

impl FixedStamps {
    /// Build from raw nanosecond values.
    #[must_use]
    pub fn new(nanos: Vec<u128>) -> Self {
        Self {
            remaining: std::sync::Mutex::new(nanos.into_iter()),
        }
    }
}

impl StampSource for FixedStamps {
    fn now(&self) -> Option<Stamp> {
        self.remaining
            .lock()
            .map_or(None, |mut it| it.next().map(Stamp::from_nanos))
    }
}

/// One line of the sink.
///
/// # The payload is stored verbatim, as a JSON string
///
/// Round-tripping it through `serde_json::Value` would **sort the keys** — the
/// default map is a `BTreeMap` — and normalise number formatting. That is an
/// instrument altering the subject's data, inside a tool whose entire product
/// is reported facts, and ADR-0011 §7a's falsification table explicitly expects
/// a malformed payload to *"land in the file exactly as it lands at the
/// receiver"*.
///
/// Escaping it into a JSON string keeps it byte-identical, guarantees the
/// one-record-per-line frame whatever whitespace the payload arrived with, and
/// works for a payload that is not valid JSON at all.
///
/// **The cost is a second parse in the reader, and it is real** — ADR-0011
/// §7a's retention table measures read-and-parse as the whole cost of a sink
/// listing. [`Record::session`] and [`Record::event`] are hoisted out for that
/// reason, so the common reads need only the outer parse.
///
/// # The hoisted fields are a reading, not a second source of truth
///
/// They are what the writer read out of the payload at write time, written once
/// in the same operation from the same parse. They cannot disagree with the
/// payload when written. If a hand-edited file ever makes them disagree, the
/// payload is the source and the hoist is the stale copy.
#[derive(Debug, Clone, Serialize)]
pub struct Record<'a> {
    /// Contract version, so a reader knows what it is holding.
    #[serde(rename = "v")]
    pub contract: &'a str,

    /// The declared identity **as the writer received it**.
    ///
    /// Recorded rather than assumed equal to what was installed, because
    /// install and write are two instruments answering one question and the
    /// channel between them can alter the value in transit — the six-for-six
    /// population in ADR-0002 §7. A disagreement is then visible in the record
    /// instead of being inferred from a missing file.
    pub identity: &'a str,

    /// Hoisted from the payload. See the type docs.
    pub session: &'a str,

    /// Hoisted from the payload: `hook_event_name`. `None` when the payload
    /// did not carry one — absent, never guessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<&'a str>,

    /// The authored stamp, decimal nanoseconds as a string. `None` when the
    /// clock could not be read; absent rather than zero.
    #[serde(rename = "stamp_ns", skip_serializing_if = "Option::is_none")]
    pub stamp: Option<String>,

    /// The hook payload, verbatim.
    pub payload: &'a str,
}
