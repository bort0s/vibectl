//! The read projection of a manifest, and the small value types it is built
//! from.
//!
//! Everything here is a *read* type. [`Manifest`] deliberately has no path back
//! to TOML — no `Display`, no `to_toml`, no `Serialize` wired to a TOML
//! serializer. Writes go through [`crate::ManifestDocument`], which is the only
//! type in the crate that can produce a `.vibe/project.toml`. That asymmetry is
//! what makes unknown-key preservation a property of the API rather than a rule
//! people have to remember (ADR-0002 §5).

mod detected;
mod manifest;
mod status;

pub use detected::{Candidate, Confidence, Detected, UnknownReason};
pub use manifest::{
    Agents, Context, Deploy, Manifest, Project, Repo, Stack, UnknownKey, ValueKind,
};
pub use status::{Status, Visibility};
