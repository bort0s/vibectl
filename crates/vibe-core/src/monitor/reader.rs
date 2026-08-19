//! Reading the sink: a **partial order**, never a sequence.
//!
//! ADR-0011 §7a measured that **no event type carries a timestamp**. Across all
//! observed types the payload offers `session_id`, `agent_id`, `prompt_id`,
//! `turn_id`, `message_id`, `tool_use_id`, `index` and `duration_ms`, and
//! nothing naming a point in time. Every stamp in the sink was authored by the
//! writer.
//!
//! So the merge key across files is a value the writer invents, and that is
//! constraint 5 pointed at **ordering** rather than at a value — the more
//! dangerous target, because a wrong value is a wrong field while a wrong order
//! is a **plausible history**. Clock skew between hook processes, or a wall
//! clock stepping under NTP, silently reorders events into a sequence that
//! reads perfectly.
//!
//! # The contract
//!
//! **The payload orders records. The stamp is a fallback. Where neither orders
//! two records, this module says so rather than presenting one of them first.**
//!
//! What the payload supports at no cost and with no clock:
//!
//! - `session_id` **groups**, and `agent_id` groups within it — neither orders.
//! - `tool_use_id` **pairs** a `PreToolUse` with its `PostToolUse`.
//! - the **lifecycle** constrains the ends: `SessionStart` precedes everything
//!   in its session, `SessionEnd` follows everything.
//! - `index` sequences `MessageDisplay` deltas inside one message.
//!
//! # What this module deliberately does not do
//!
//! - **No consumption tracking and no watermark.** The reader is stateless: it
//!   walks the sink and reads everything, every time. A watermark is a second
//!   artifact that must be kept equal to the files — the shape ADR-0010 §3
//!   rejected — and its staleness is silent in the dangerous direction, because
//!   a watermark ahead of the truth *hides records*.
//! - **No deletion.** ADR-0001 §3 enforces constraint 2 by the absence of
//!   `FileOp::Delete`. Prunability is computed and displayed; the user deletes.
//! - **No display.** ADR-0011 §6 constrains it and §8 leaves it open.
//!
//! # The stamp's standing here, restated because it is weaker than it looks
//!
//! Within one file the records come from **many invocations of one hook**, each
//! a separate process reading the same wall clock. §7a's *"stamps must be
//! non-decreasing inside one file"* therefore rests on those appends being
//! sequential — which is the intra-agent concurrency question that is a
//! **declared limit**: measured on 2.1.234 against a fixture that does build
//! six parallel tool calls for one agent, where no two invocations sharing a key
//! overlapped, and not proven impossible. So a decreasing stamp inside one file
//! is reported as an observation ([`ClockStep`]) and not used to reorder
//! anything — which is the reading that survives the limit turning out to be
//! wrong — and the module claims no cross-file stamp comparability at all.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::identity::{AgentComponent, SEPARATOR, SessionComponent, WriterIdentity, file_key};
use super::sink::{ReadRecord, SinkRead, TailState, read_file};

/// What a filename claims about the records inside it.
///
/// Parsed by splitting on [`SEPARATOR`], which is unambiguous because no
/// component may contain it: two fields is a session-level file, three is an
/// agent-level one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "attribution", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Attribution {
    /// The name parsed and every component validated.
    Attributed {
        session: SessionComponent,
        agent: Option<AgentComponent>,
        identity: WriterIdentity,
    },
    /// The name is in the sink and this module cannot attribute it.
    ///
    /// **A state, not an error** (ADR-0011 §7a). §7 permits hand-installed
    /// hooks, so a file whose writer omitted or mangled the declared identity is
    /// the ordinary case rather than the exotic one. Its records are still read:
    /// the events are real and the session is named in every payload; only the
    /// *source* is unknown. Discarding them would lose real events, guessing a
    /// source would invent one, and calling it an error would say something
    /// failed when nothing did.
    Unattributed { reason: &'static str },
}

impl Attribution {
    /// Stable key.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            Attribution::Attributed { .. } => "attributed",
            Attribution::Unattributed { .. } => "unattributed",
        }
    }

    /// Parse a sink filename.
    #[must_use]
    pub fn of(path: &Path) -> Self {
        let unattributed = |reason| Attribution::Unattributed { reason };
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return unattributed("filename is not valid UTF-8");
        };
        let Some(stem) = name.strip_suffix(".jsonl") else {
            return unattributed("not a .jsonl file");
        };
        let parts: Vec<&str> = stem.split(SEPARATOR).collect();
        let (session, agent, identity) = match parts.as_slice() {
            [s, i] => (*s, None, *i),
            [s, a, i] => (*s, Some(*a), *i),
            _ => {
                return unattributed(
                    "name is not <session>__<identity> or <session>__<agent>__<identity>",
                );
            }
        };
        let Ok(session) = SessionComponent::parse(session) else {
            return unattributed("session component is not a valid path component");
        };
        let agent = match agent.map(AgentComponent::parse) {
            None => None,
            Some(Ok(a)) => Some(a),
            Some(Err(_)) => return unattributed("agent component is not a valid path component"),
        };
        let Ok(identity) = WriterIdentity::parse(identity) else {
            return unattributed("identity component is not a valid path component");
        };
        Attribution::Attributed {
            session,
            agent,
            identity,
        }
    }
}

/// A place where the filename and the records inside it disagree.
///
/// **Reported, never resolved.** The filename is what the writer chose; the
/// payload is what the agent reported. When they differ, one of them is wrong
/// and this module cannot tell which — picking either would be inventing the
/// answer that decides which session a record belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Disagreement {
    pub path: PathBuf,
    /// What the filename claims.
    pub filename_says: String,
    /// What a record inside claims.
    pub record_says: String,
    /// Which field disagreed.
    pub field: &'static str,
    /// How many records in this file disagree.
    pub records: usize,
}

/// A decreasing stamp inside one file.
///
/// One writer identity appends to one file, so a stamp that goes backwards is
/// **direct evidence the clock stepped** — a reported fact rather than an
/// inference, needing no cross-file reasoning.
///
/// It does **not** reorder anything. See the module docs for why the premise is
/// weaker than §7a states it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClockStep {
    pub path: PathBuf,
    pub from: String,
    pub to: String,
}

/// Why two records are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OrderBasis {
    /// `SessionStart` precedes everything; `SessionEnd` follows everything.
    Lifecycle,
    /// One `tool_use_id`: `PreToolUse` precedes its `PostToolUse`.
    ToolPair,
    /// `index` within one `message_id`.
    MessageIndex,
}

/// Whether two records can be ordered, and on what evidence.
///
/// **`Unordered` is a verdict, not a failure.** ADR-0011 §7a: where two records
/// are unordered by both the payload and the stamps, the reader must *say so
/// rather than present one*, and the display inherits that as a third state one
/// level down — *ordered* versus *unordered with respect to each other*, with
/// the second never borrowing the appearance of the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "order", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecordOrder {
    Before {
        basis: OrderBasis,
    },
    After {
        basis: OrderBasis,
    },
    /// Neither the payload nor a same-file stamp orders these two.
    Unordered,
}

impl RecordOrder {
    /// Stable key.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            RecordOrder::Before { .. } => "before",
            RecordOrder::After { .. } => "after",
            RecordOrder::Unordered => "unordered",
        }
    }

    /// Whether an order was established at all.
    #[must_use]
    pub fn is_ordered(&self) -> bool {
        !matches!(self, RecordOrder::Unordered)
    }
}

/// One record, with the fields ordering needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// Which file it came from, so a reader can say where.
    pub path: PathBuf,
    /// Position within that file, which is the one order that is not authored:
    /// a file is appended to, so later in the file is later in the file.
    pub line: usize,
    pub session: Option<String>,
    pub agent: Option<String>,
    pub event: Option<String>,
    pub prompt_id: Option<String>,
    pub tool_use_id: Option<String>,
    pub message_id: Option<String>,
    pub index: Option<i64>,
    pub stamp_ns: Option<String>,
}

/// Order two entries by the payload, and only by the payload.
///
/// Returns [`RecordOrder::Unordered`] whenever the evidence does not reach —
/// which is most pairs, and saying so is the point.
#[must_use]
pub fn order(a: &Entry, b: &Entry) -> RecordOrder {
    // Different sessions are not comparable at all.
    if a.session != b.session {
        return RecordOrder::Unordered;
    }

    // Lifecycle bounds the ends of a session.
    let ends = |e: &Entry| match e.event.as_deref() {
        Some("SessionStart") => Some(-1i8),
        Some("SessionEnd") => Some(1),
        _ => None,
    };
    match (ends(a), ends(b)) {
        (Some(-1), Some(-1)) | (Some(1), Some(1)) => {}
        (Some(-1), _) | (_, Some(1)) => {
            return RecordOrder::Before {
                basis: OrderBasis::Lifecycle,
            };
        }
        (_, Some(-1)) | (Some(1), _) => {
            return RecordOrder::After {
                basis: OrderBasis::Lifecycle,
            };
        }
        _ => {}
    }

    // A tool pair: same id, Pre before Post.
    // Written without a let-chain: they are stable from 1.88 and this
    // workspace pins 1.85.0, which the MSRV job caught.
    if matches!((&a.tool_use_id, &b.tool_use_id), (Some(ia), Some(ib)) if ia == ib) {
        let phase = |e: &Entry| match e.event.as_deref() {
            Some("PreToolUse") => Some(0u8),
            Some("PostToolUse") => Some(1),
            _ => None,
        };
        match (phase(a), phase(b)) {
            (Some(0), Some(1)) => {
                return RecordOrder::Before {
                    basis: OrderBasis::ToolPair,
                };
            }
            (Some(1), Some(0)) => {
                return RecordOrder::After {
                    basis: OrderBasis::ToolPair,
                };
            }
            _ => {}
        }
    }

    // Deltas within one message.
    let same_message = matches!(
        (&a.message_id, &b.message_id),
        (Some(ma), Some(mb)) if ma == mb
    );
    if let (true, Some(ixa), Some(ixb)) = (same_message, a.index, b.index) {
        if ixa == ixb {
            return RecordOrder::Unordered;
        }
        return if ixa < ixb {
            RecordOrder::Before {
                basis: OrderBasis::MessageIndex,
            }
        } else {
            RecordOrder::After {
                basis: OrderBasis::MessageIndex,
            }
        };
    }

    RecordOrder::Unordered
}

/// Whether a set of records can be presented as a sequence.
///
/// **Derived from [`order`], never maintained beside it.** A second field kept
/// equal to the primitive is the shape ADR-0010 §3 rejected, and its staleness
/// would be silent in the dangerous direction — a listing that has gone stale
/// toward `FullyOrdered` reads as a history.
///
/// # Why the listing carries this at all, when the primitive already answers it
///
/// The per-pair verdict is the contract and it is complete: ask about any two
/// records and you are told. But **a listing containing unordered pairs and not
/// saying so reads as a sequence** — records come out in some order, and an
/// order that is merely an artifact of iteration is indistinguishable from one
/// that was established.
///
/// That is ADR-0010 §5's argument exactly: the state is shown **by default**
/// because the failure is *"I didn't notice"*, and a property available on
/// request does not guard against not noticing. The two are layers rather than
/// alternatives — the listing says *parts of this are unordered*, the primitive
/// says *which parts*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "sequencing", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Sequencing {
    /// Fewer than two records: nothing to order, and not the same as ordered.
    Trivial,
    /// Every pair is ordered by the payload. This set may be presented as a
    /// sequence.
    FullyOrdered,
    /// At least one pair is unordered. Presenting this as a sequence would be
    /// presenting a plausible history.
    PartlyUnordered { unordered_pairs: usize },
}

impl Sequencing {
    /// Stable key.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            Sequencing::Trivial => "trivial",
            Sequencing::FullyOrdered => "fully_ordered",
            Sequencing::PartlyUnordered { .. } => "partly_unordered",
        }
    }

    /// Whether these records may be rendered as a sequence.
    ///
    /// `Trivial` answers **false**: zero or one record is not an ordered set,
    /// and saying otherwise would let an empty listing license a sequence.
    #[must_use]
    pub fn may_be_presented_as_a_sequence(&self) -> bool {
        matches!(self, Sequencing::FullyOrdered)
    }
}

/// Derive [`Sequencing`] over a set of entries.
///
/// O(n²) in the number of entries and deliberately so: the question is about
/// pairs, and there is no cheaper honest answer while the ordering relation is
/// partial rather than a key to sort on.
#[must_use]
pub fn sequencing(entries: &[Entry]) -> Sequencing {
    if entries.len() < 2 {
        return Sequencing::Trivial;
    }
    let mut unordered_pairs = 0usize;
    for (i, a) in entries.iter().enumerate() {
        for b in &entries[i + 1..] {
            if !order(a, b).is_ordered() {
                unordered_pairs += 1;
            }
        }
    }
    if unordered_pairs == 0 {
        Sequencing::FullyOrdered
    } else {
        Sequencing::PartlyUnordered { unordered_pairs }
    }
}

/// Everything read from a sink, in one stateless pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SinkListing {
    /// Per file, in path order.
    pub files: Vec<FileView>,
    /// Files this module could not attribute. Their records are still in
    /// [`FileView::entries`].
    pub unattributed: usize,
    /// Filename-versus-payload conflicts, reported and unresolved.
    pub disagreements: Vec<Disagreement>,
    /// Backwards stamps inside a single file.
    pub clock_steps: Vec<ClockStep>,
    /// Whether the records across the whole sink may be presented as a
    /// sequence. Derived from [`order`]; see [`Sequencing`].
    pub sequencing: Sequencing,
    /// Declared identities that fold to one filename key, across the files
    /// present. A configuration fault surfacing in the records is late — §7a
    /// puts the real check on the config — but a reader that sees it must say
    /// so rather than merging the records silently.
    pub identity_collisions: Vec<String>,
}

/// One file in the sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileView {
    pub path: PathBuf,
    pub attribution: Attribution,
    pub tail: TailState,
    pub entries: Vec<Entry>,
    /// Lines that were whole but did not parse.
    pub unparseable: usize,
}

/// Walk a sink directory and read every file, every time.
///
/// Unreadable files are reported as [`SinkRead::Unreadable`] and skipped from
/// [`SinkListing::files`]; the count is not folded into anything, because a file
/// that could not be read and a file with no records are different facts.
///
/// # Errors
///
/// Returns the directory read error when the sink itself cannot be enumerated —
/// which is distinct from an empty sink, and must not render the same.
pub fn read_sink(dir: &Path) -> Result<(SinkListing, Vec<PathBuf>), std::io::Error> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .collect();
    paths.sort();

    let mut files = Vec::new();
    let mut unreadable = Vec::new();
    let mut disagreements = Vec::new();
    let mut clock_steps = Vec::new();
    let mut unattributed = 0usize;
    let mut by_key: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for path in paths {
        let SinkRead::Read(file) = read_file(&path) else {
            unreadable.push(path);
            continue;
        };
        let attribution = Attribution::of(&path);
        if let Attribution::Attributed { identity, .. } = &attribution {
            by_key
                .entry(identity.file_key())
                .or_default()
                .push(identity.as_str().to_owned());
        } else {
            unattributed += 1;
        }

        let mut entries = Vec::new();
        let mut unparseable = 0usize;
        let mut previous: Option<u128> = None;
        for (line, record) in file.records.iter().enumerate() {
            match record {
                ReadRecord::Unparseable { .. } => unparseable += 1,
                ReadRecord::Parsed {
                    session,
                    event,
                    stamp_ns,
                    payload,
                } => {
                    let entry = Entry::from_record(
                        &path,
                        line,
                        session.as_deref(),
                        event.as_deref(),
                        stamp_ns.as_deref(),
                        payload.as_deref(),
                    );
                    if let Some(now) = entry.stamp_ns.as_ref().and_then(|s| s.parse::<u128>().ok())
                    {
                        if previous.is_some_and(|prev| now < prev) {
                            let prev = previous.unwrap_or(now);
                            clock_steps.push(ClockStep {
                                path: path.clone(),
                                from: prev.to_string(),
                                to: now.to_string(),
                            });
                        }
                        previous = Some(now);
                    }
                    if let Attribution::Attributed { session, agent, .. } = &attribution {
                        check_disagreement(
                            &path,
                            "session",
                            session.as_str(),
                            entry.session.as_deref(),
                            &mut disagreements,
                        );
                        check_disagreement(
                            &path,
                            "agent",
                            agent.as_ref().map_or("", AgentComponent::as_str),
                            entry.agent.as_deref().or(Some("")),
                            &mut disagreements,
                        );
                    }
                    entries.push(entry);
                }
            }
        }

        files.push(FileView {
            path,
            attribution,
            tail: file.tail,
            entries,
            unparseable,
        });
    }

    let identity_collisions = by_key
        .into_iter()
        .filter(|(_, v)| {
            let mut distinct: Vec<&String> = v.iter().collect();
            distinct.sort();
            distinct.dedup();
            distinct.len() > 1
        })
        .map(|(k, _)| k)
        .collect();

    let all: Vec<Entry> = files
        .iter()
        .flat_map(|f| f.entries.iter().cloned())
        .collect();
    let sequencing = sequencing(&all);

    Ok((
        SinkListing {
            files,
            sequencing,
            unattributed,
            disagreements,
            clock_steps,
            identity_collisions,
        },
        unreadable,
    ))
}

fn check_disagreement(
    path: &Path,
    field: &'static str,
    filename_says: &str,
    record_says: Option<&str>,
    out: &mut Vec<Disagreement>,
) {
    let record_says = record_says.unwrap_or("");
    if record_says == filename_says {
        return;
    }
    if let Some(existing) = out
        .iter_mut()
        .find(|d| d.path == path && d.field == field && d.record_says == record_says)
    {
        existing.records += 1;
        return;
    }
    out.push(Disagreement {
        path: path.to_path_buf(),
        filename_says: filename_says.to_owned(),
        record_says: record_says.to_owned(),
        field,
        records: 1,
    });
}

impl Entry {
    /// Build an entry, preferring the **payload** over the writer's hoist.
    ///
    /// The hoist and the payload cannot disagree at write time — they are
    /// written in one operation from one parse — but a hand-edited file can make
    /// them disagree, and then the payload is the source and the hoist is the
    /// stale copy. Reading the payload first means this module never depends on
    /// that having gone right.
    fn from_record(
        path: &Path,
        line: usize,
        hoisted_session: Option<&str>,
        hoisted_event: Option<&str>,
        stamp_ns: Option<&str>,
        payload: Option<&str>,
    ) -> Self {
        let parsed: Option<serde_json::Map<String, serde_json::Value>> = payload
            .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
            .and_then(|v| v.as_object().cloned());
        let field = |k: &str| {
            parsed
                .as_ref()
                .and_then(|o| o.get(k))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        Self {
            path: path.to_path_buf(),
            line,
            session: field("session_id").or_else(|| hoisted_session.map(str::to_owned)),
            agent: field("agent_id"),
            event: field("hook_event_name").or_else(|| hoisted_event.map(str::to_owned)),
            prompt_id: field("prompt_id"),
            tool_use_id: field("tool_use_id"),
            message_id: field("message_id"),
            index: parsed
                .as_ref()
                .and_then(|o| o.get("index"))
                .and_then(serde_json::Value::as_i64),
            stamp_ns: stamp_ns.map(str::to_owned),
        }
    }
}

/// Whether a declared identity string collides with another under
/// [`file_key`]. Exposed so a caller can ask the same question of a *config*,
/// which is where §7a puts the real check.
#[must_use]
pub fn collides(a: &str, b: &str) -> bool {
    file_key(a) == file_key(b)
}
