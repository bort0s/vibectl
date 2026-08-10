//! Progress and diagnostics, pushed into a sink the caller supplies.
//!
//! This crate cannot print. It also does not return an iterator of events or
//! spawn a thread to deliver them: it calls [`Reporter::event`] as things
//! happen, and the caller decides what that means — a progress bar on stderr, a
//! `window.emit` to a webview, or nothing at all.
//!
//! Nothing here holds a preformatted sentence. A [`Diagnostic`] is a stable
//! `code` plus named `params`, so a consumer renders its own wording (and its
//! own language) instead of reimplementing this crate's message catalogue.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Serialize;

/// A sink for progress events.
///
/// **Event ordering is not guaranteed.** Once `scan` runs detectors in
/// parallel (P2), `event` is called from multiple worker threads, so a
/// consumer must not assume an event about one project precedes another about
/// the same project. Build state keyed by subject, not a state machine driven
/// by arrival order.
pub trait Reporter: Send + Sync {
    fn event(&self, ev: Event);

    /// Polled at operation boundaries during long-running work.
    ///
    /// Cancellation is a *success* outcome, never an error: a GUI Cancel button
    /// must not surface as an error dialog, and work already completed must
    /// survive. See [`crate::ApplyOutcome`] and ADR-0005 §2.
    fn should_cancel(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Event {
    ApplyStarted {
        ops: usize,
    },
    OpApplied {
        /// Lossy display form. Nothing addresses anything by this string.
        path: String,
    },
    ApplyFinished {
        applied: usize,
        /// Milliseconds, not `Duration`. `Duration` serialises as
        /// `{"secs":0,"nanos":123000000}`, which is hostile to both `jq` and a
        /// webview (ADR-0005 §5).
        elapsed_ms: u64,
    },
    Diagnostic(Diagnostic),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Severity {
    Note,
    Warn,
}

/// Structured, not prose. `code` is stable and safe to branch on; `params` are
/// named interpolation values for whatever sentence the consumer writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

impl Diagnostic {
    #[must_use]
    pub fn warn(code: &'static str) -> Self {
        Self {
            severity: Severity::Warn,
            code,
            subject: None,
            params: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    #[must_use]
    pub fn with_param(mut self, key: &str, value: impl Into<String>) -> Self {
        self.params.insert(key.to_owned(), value.into());
        self
    }
}

/// Emitted per manifest when a file declares a higher minor than this build
/// knows. `vibectl` coalesces these into a single line: a scan over 30
/// forward-versioned manifests must not print 30 warnings (ADR-0002 §3).
pub const W_SCHEMA_MINOR_NEWER: &str = "VIBE_W_SCHEMA_MINOR_NEWER";

/// Discards everything. The default for callers that want no progress.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullReporter;

impl Reporter for NullReporter {
    fn event(&self, _ev: Event) {}
}

/// Accumulates events. For tests, and for `--json`, where the whole run is
/// serialised at the end rather than streamed.
#[derive(Debug, Default)]
pub struct CollectingReporter {
    events: Mutex<Vec<Event>>,
}

impl CollectingReporter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A snapshot of everything seen so far.
    ///
    /// Lock poisoning is recovered from rather than propagated. In a CLI a
    /// panic ends the process and poisoning is unobservable; in a desktop app
    /// the process survives, and one panicking worker would otherwise brick
    /// every later call for the life of the process (ADR-0005 §7).
    #[must_use]
    pub fn events(&self) -> Vec<Event> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                Event::Diagnostic(d) => Some(d),
                _ => None,
            })
            .collect()
    }
}

impl Reporter for CollectingReporter {
    fn event(&self, ev: Event) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(ev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collecting_reporter_survives_a_poisoned_lock() {
        let rep = std::sync::Arc::new(CollectingReporter::new());
        rep.event(Event::ApplyStarted { ops: 1 });

        let r2 = std::sync::Arc::clone(&rep);
        let _ = std::thread::spawn(move || {
            let _guard = r2.events.lock().unwrap();
            panic!("poison the lock");
        })
        .join();

        // The whole point: a panicked worker must not brick the reporter.
        assert_eq!(rep.events().len(), 1);
        rep.event(Event::ApplyStarted { ops: 2 });
        assert_eq!(rep.events().len(), 2);
    }
}
