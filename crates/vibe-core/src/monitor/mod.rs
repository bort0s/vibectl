//! Live agent monitoring: the **writer** half.
//!
//! ADR-0011 is the design. This module is the transport decided in §7a and
//! nothing else — no display (§6 constrains it and §8 leaves it open), and no
//! reader beyond what the controls in §9 assert on.
//!
//! # What the transport is, and why the decision reversed
//!
//! A **file**, appended to by a `command` hook in the `args` exec form.
//! `http` was chosen first, on a property no other variant has — the receiving
//! side holds independent evidence of its own liveness, so *"no events"* splits
//! into *"the receiver was up and heard nothing"* and *"the receiver was
//! down"*. Costing it reversed the choice, and the reversal is the more useful
//! record:
//!
//! 1. A receiver's evidence covers only the interval it has been continuously
//!    up, because **a process that is down cannot record its own downtime** —
//!    so it needs a durable coverage log regardless, which puts a listener in
//!    front of a file rather than replacing one.
//! 2. Give both transports that log and the residual is identical.
//! 3. Then the asymmetry runs the other way: *"receiver down"* is a failure
//!    mode that exists **only because there is a receiver**. During downtime a
//!    hook still appends to a file; with `http` those events are lost — and
//!    sleep, reboot and crash are exactly the long-silence case the monitor
//!    exists for.
//! 4. So the deciding property was information about the **receiver**, not
//!    about the subject. *Information whose sole purpose is explaining your own
//!    failure mode is not a reason to adopt the failure mode.*
//!
//! # What this module does not fix, stated so the label cannot overreach
//!
//! **A file that stops growing is indistinguishable from a quiet agent.**
//! Neither transport ever fixed that. It is §7's hazard; it is answered by the
//! per-session wiring proof and by §5's liveness check failing in *different*
//! directions, and the transport decision was never capable of touching it.
//!
//! # The channel is ambiguous in both directions
//!
//! Silence overstates stoppage; an unclosed event overstates activity. §5's
//! round-2 retraction is total: **an unmatched `tool_use_id` with the reporter
//! alive supports nothing about the present** — a denied tool leaves its id
//! open permanently, in a session that has already stopped. Nothing in this
//! module may be read as reporting what an agent is doing now.
//!
//! # Layout
//!
//! - [`identity`] — the declared identity and the session, validated as path
//!   components; the collision check that runs over a *config*.
//! - [`record`] — one line of the sink, and the authored stamp with what it
//!   does and does not claim.
//! - [`writer`] — the append, and the failure taxonomy that keeps a failed
//!   write from becoming silent non-delivery.
//! - [`sink`] — the tail check and prunability, built only as far as §9's
//!   controls (b) and (f) reach.
//! - [`settings`] — editing `settings.json`, a file vibe does not own: what
//!   the write preserves, and why re-install rather than first install is the
//!   case it is built around.

pub mod identity;
pub mod reader;
pub mod record;
pub mod settings;
pub mod sink;
pub mod writer;

pub use identity::{
    AGENT_MAX_LEN, AgentComponent, ComponentRejection, IDENTITY_MAX_LEN, IdentityCollision,
    SEPARATOR, SESSION_MAX_LEN, SessionComponent, WriterIdentity, collisions, file_key,
};
pub use reader::{
    Attribution, ClockStep, Disagreement, Entry, FileView, OrderBasis, RecordOrder, Sequencing,
    SinkListing, collides, order, read_sink, sequencing,
};
pub use record::{CONTRACT_VERSION, FixedStamps, Record, Stamp, StampSource, SystemStamps};
pub use settings::{
    HOOK_TIMEOUT_FLOOR_SECS, HOOK_TIMEOUT_MULTIPLIER, HOOK_TIMEOUT_SECS, HookSpec,
    INSTALLED_EVENTS, InstallOutcome, MATCH_ALL, SettingsDocument, SettingsRefusal, install,
    read_document,
};
pub use sink::{
    NotPrunableReason, Prunability, ReadRecord, SESSION_END_EVENT, SinkFile, SinkRead, TailState,
    read_file,
};
pub use writer::{IoFailure, PayloadRefusal, WriteOutcome, WriteStage, Writer};
