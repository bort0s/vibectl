//! Generating `CLAUDE.md`, `AGENTS.md` and `README.md` from the manifest.
//!
//! A rendered file is **entirely** `vibe`'s: there are no user-editable regions
//! inside it, and nothing outside it is touched. [`marker`] carries the proof of
//! both halves — that we wrote it, and that it has not changed since — in the
//! file itself rather than in a sidecar record.
//!
//! The rule that makes `README.md` safe to have as a target: a file with no
//! marker is [`RenderState::Foreign`], and **`--force` does not move it**. The
//! dangerous case was never "the user edited our README", it is "the user has a
//! README and we never wrote it". See ADR-0007.

pub mod marker;

mod engine;

pub use engine::{RenderTarget, render_body};
pub use marker::{MARKER_VERSION, RenderState};
