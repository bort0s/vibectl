//! Injectable configuration. No global state, no ambient clock.
//!
//! [`Config`] is constructed *by* the caller, so it gets `Default` plus `with_*`
//! setters rather than `#[non_exhaustive]` — a future field must not break every
//! existing struct literal (ADR-0005 §3). A desktop consumer builds one
//! explicitly instead of calling [`Config::discover`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Today's date, injectable so tests are deterministic.
///
/// A trait rather than a call to the system clock inside `plan_new`, because a
/// scaffolded manifest records `created` and a test that asserts on the
/// rendered file would otherwise fail once a day, at midnight UTC, on someone
/// else's machine.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// `YYYY-MM-DD`, UTC.
    fn today_utc(&self) -> String;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn today_utc(&self) -> String {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let days = i64::try_from(secs / 86_400).unwrap_or(0);
        let (y, m, d) = civil_from_days(days);
        format!("{y:04}-{m:02}-{d:02}")
    }
}

/// A fixed date. For tests.
#[derive(Debug, Clone)]
pub struct FixedClock(pub String);

impl Clock for FixedClock {
    fn today_utc(&self) -> String {
        self.0.clone()
    }
}

/// Days since the Unix epoch to a civil `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`, which is exact for the proleptic
/// Gregorian calendar. Implemented here rather than pulled in as a dependency:
/// the whole date requirement of this project is stamping `created` on a new
/// manifest, and `chrono`/`time` are not on the approved dependency list.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

#[derive(Debug, Clone)]
pub struct Config {
    roots: Vec<PathBuf>,
    clock: Arc<dyn Clock>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            clock: Arc::new(SystemClock),
        }
    }
}

impl Config {
    /// Configuration derived from the OS-appropriate directories.
    ///
    /// Infallible today because the only thing it discovers is the clock;
    /// cache and config paths arrive in P3. It returns `Self` rather than
    /// `Result<Self>` for now, and the signature will become fallible when it
    /// starts touching the filesystem.
    #[must_use]
    pub fn discover() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.roots.push(root.into());
        self
    }

    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    #[must_use]
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    #[must_use]
    pub fn today_utc(&self) -> String {
        self.clock.today_utc()
    }

    /// Where the global cache will live. Regenerable, never authoritative — if
    /// it is missing, `vibe scan` rebuilds it (P3).
    #[must_use]
    pub fn cache_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "vibe").map(|d| d.cache_dir().to_path_buf())
    }
}

/// The manifest's location within a project directory.
#[must_use]
pub fn manifest_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".vibe").join("project.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_anchors() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 30 years, of which 1972..=1996 contribute 7 leap days.
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        // +31 (Jan) +29 (Feb 2000, a leap year) = 60 days.
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(10_956), (1999, 12, 31));
    }

    #[test]
    fn civil_from_days_is_monotonic_and_well_formed() {
        let mut prev = civil_from_days(0);
        for day in 1..=(365 * 80) {
            let cur = civil_from_days(day);
            assert!(cur > prev, "day {day}: {cur:?} should follow {prev:?}");
            assert!((1..=12).contains(&cur.1), "bad month at day {day}: {cur:?}");
            assert!((1..=31).contains(&cur.2), "bad day at day {day}: {cur:?}");
            prev = cur;
        }
    }

    #[test]
    fn system_clock_produces_a_parseable_iso_date() {
        let today = SystemClock.today_utc();
        assert_eq!(today.len(), 10, "expected YYYY-MM-DD, got {today}");
        let parts: Vec<_> = today.split('-').collect();
        assert_eq!(parts.len(), 3);
        let year: i64 = parts[0].parse().expect("year");
        assert!((2020..2200).contains(&year), "implausible year: {today}");
    }
}
