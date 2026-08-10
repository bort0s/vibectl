//! Reading and writing `.vibe/project.toml`.
//!
//! The one rule that shapes this module: **the typed struct is not the write
//! source.** [`crate::Manifest`] is a read projection with no path back to
//! TOML. Every mutation goes through [`ManifestDocument`], which holds the live
//! `toml_edit` document that was read off disk and edits addressed keys inside
//! it. Nothing ever rebuilds the document from a struct, so comments, key
//! order, whitespace, array style, and keys this build has never heard of
//! survive by construction rather than by anyone remembering to preserve them.
//!
//! That is the difference between this and a `serde` round-trip, and it is the
//! reason `toml_edit` is a hard requirement rather than a preference. See
//! ADR-0002 §5.

mod document;
mod parse;
mod version;

pub use document::{EditReason, FieldEdit, ManifestDocument};
pub use version::{Compat, ParseSchemaVersionError, SCHEMA_MAJOR, SCHEMA_MINOR, SchemaVersion};
