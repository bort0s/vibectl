//! Core library for `vibectl` — a registry for the half-finished projects a
//! developer already has on disk.
//!
//! # Crate contract
//!
//! This crate is the reusable engine: manifest types, parsing, project
//! detection, and the render engine. Two rules define its boundary. Both are
//! checked in CI; neither is fully enforced by `cargo build` alone, so the
//! exact coverage is worth stating.
//!
//! 1. **No terminal I/O.** Nothing here writes to stdout or stderr. Results and
//!    diagnostics are *returned*, never printed — which is what allows a future
//!    Tauri frontend to consume the same code paths the CLI uses.
//!
//!    The `deny` attributes below fail the build **under `cargo clippy`**, not
//!    under a plain `cargo build`: these are clippy lints, and rustc ignores
//!    them. CI runs clippy with `-D warnings`, so the rule holds there. They
//!    also only cover the `print!`/`println!` macro family — an explicit
//!    `writeln!(std::io::stdout(), ..)` slips past them and is caught by review
//!    alone.
//! 2. **No argument parsing.** `clap` belongs to the `vibectl` crate. Nothing
//!    in the language prevents adding it here, so this half of the boundary is
//!    asserted by a CI step that inspects `cargo tree -p vibe-core`.
//!
//! # Mutation model
//!
//! Nothing in this crate writes to the filesystem except [`Registry::apply`],
//! and the only thing it accepts is a [`WritePlan`] produced by a `plan_*`
//! method. `--dry-run` is therefore not a flag this crate knows about: it is
//! the caller declining to make the second call. See `docs/adr/0001` and
//! `docs/adr/0005`.

#![deny(clippy::print_stdout, clippy::print_stderr)]

pub mod agents;
pub mod cache;
pub mod config;
pub mod detect;
pub mod error;
pub mod exec;
pub mod manifest;
pub mod model;
pub mod ops;
pub mod plan;
pub mod registry;
pub mod report;
pub mod scan;
pub mod url;
pub mod view;
pub mod walk;

pub use agents::{AgentState, LockLoad, Lockfile};
pub use cache::{Cache, CacheLoad};
pub use config::Config;
pub use detect::{Detection, Detector, DetectorId, Evidence, FieldPath, Specificity, Suggestion};
pub use error::{CoreError, ErrorPayload};
pub use exec::{NoRunner, ProcessRunner, SystemRunner};
pub use manifest::{FieldEdit, ManifestDocument, SchemaVersion};
pub use model::{Confidence, Detected, Manifest, Status, UnknownReason, Visibility};
pub use ops::SyncNotes;
pub use plan::{ApplyOutcome, ApplyReport, FileOp, PlanIntent, WritePlan};
pub use registry::{NewRequest, Registry};
pub use report::{CollectingReporter, Diagnostic, Event, NullReporter, Reporter, Severity};
pub use scan::{ScanOutcome, ScanReport, ScanRequest, ScannedProject};
pub use url::GitUrl;
pub use view::{ListReport, ProjectSummary, ProjectView, Query};
pub use walk::FileIndex;

/// Types that cross a `spawn_blocking` boundary in a desktop consumer must be
/// `Send + Sync + 'static`. ADR-0005 §8 requires this to be a compile-time
/// assertion rather than an assumption, because the day it stops being true is
/// the day someone adds an `Rc` field and nothing complains until a frontend
/// exists to complain.
const _: fn() = || {
    fn assert_send_sync_static<T: Send + Sync + 'static>() {}
    assert_send_sync_static::<Registry>();
    assert_send_sync_static::<Config>();
    assert_send_sync_static::<WritePlan>();
    assert_send_sync_static::<ApplyReport>();
    assert_send_sync_static::<Manifest>();
    assert_send_sync_static::<CoreError>();
};
