//! The read shapes behind `list` and `show`.
//!
//! The division of labour, from ADR-0004: **`list` may be stale and says so;
//! `show` never is.** `list` renders from the cache when its witness still
//! matches the manifest on disk, and re-reads when it does not. `show` always
//! re-reads. No `WritePlan` is ever built from cached content.

use serde::Serialize;

use crate::cache::CacheEntry;
use crate::error::ErrorPayload;
use crate::manifest::SchemaVersion;
use crate::model::{Manifest, Status, UnknownKey};

/// One row of `vibe list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ProjectSummary {
    pub path: String,
    pub name: String,
    pub status: String,
    pub archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deploy_url: Option<String>,
    /// True when this row came from the cache rather than a fresh read.
    ///
    /// Emitted so a consumer can tell; the value is still believed to be
    /// correct — a stale row is one whose witness matched, meaning the manifest
    /// has not changed since it was recorded.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_cache: bool,
    /// Set when the manifest exists but could not be read. The project is still
    /// listed — an error row, never a missing entry (ADR-0002 §3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
}

impl ProjectSummary {
    #[must_use]
    pub fn from_cache_entry(entry: &CacheEntry) -> Self {
        Self {
            path: entry.path.clone(),
            name: entry.name.clone(),
            status: entry.status.clone(),
            archived: entry.archived,
            runtime: entry.runtime.clone(),
            remote: entry.remote.clone(),
            last_commit: entry.last_commit.clone(),
            deploy_url: entry.deploy_url.clone(),
            from_cache: true,
            error: None,
        }
    }
}

/// Everything `vibe show` knows about one project.
///
/// Built only from a fresh read. The point of `show` is to be the thing you
/// paste at an agent, and a cached answer would undermine that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ProjectView {
    pub path: String,
    pub manifest_path: String,
    pub schema_version: SchemaVersion,
    /// How this build relates to the manifest's schema version.
    pub compat: crate::manifest::Compat,
    pub manifest: Manifest,
    /// Keys this build does not understand. Reported so the user can see what
    /// an upgrade would unlock, never round-tripped through here.
    pub unknown_keys: Vec<UnknownKey>,
}

impl ProjectView {
    #[must_use]
    pub fn status(&self) -> &Status {
        &self.manifest.project.status
    }
}

/// Filters for `vibe list`.
///
/// Caller-constructed, so `Default` plus setters rather than
/// `#[non_exhaustive]`.
#[derive(Debug, Clone, Default)]
pub struct Query {
    include_archived: bool,
    status: Option<Status>,
}

impl Query {
    /// Include archived projects. They are hidden by default — archiving means
    /// "off my desk", and a list that keeps showing them has not done its job
    /// (ADR-0002 §6).
    #[must_use]
    pub fn with_archived(mut self, include: bool) -> Self {
        self.include_archived = include;
        self
    }

    #[must_use]
    pub fn with_status(mut self, status: Option<Status>) -> Self {
        self.status = status;
        self
    }

    #[must_use]
    pub fn matches(&self, summary: &ProjectSummary) -> bool {
        if summary.archived && !self.include_archived {
            return false;
        }
        match &self.status {
            Some(want) => summary.status == want.to_string(),
            None => true,
        }
    }
}

/// The result of `vibe list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ListReport {
    pub projects: Vec<ProjectSummary>,
    /// Rows served from the cache, for the "may be stale and says so" half of
    /// the contract.
    pub from_cache: usize,
    /// How many projects were hidden because they are archived. Reported so
    /// "nothing here" is never ambiguous.
    pub archived_hidden: usize,
    /// A note about the cache, when there is one worth making.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_note: Option<String>,
}

impl ListReport {
    #[must_use]
    pub fn unreadable(&self) -> usize {
        self.projects.iter().filter(|p| p.error.is_some()).count()
    }
}
