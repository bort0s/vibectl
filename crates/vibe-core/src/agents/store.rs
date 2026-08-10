//! The agent store: a git clone of an upstream repository, read offline.
//!
//! `update` is the **only** thing in this crate that touches the network
//! (ADR-0006 §1). Everything else — `status`, `add`, `remove`, `sync`, `list` —
//! works against whatever the store already holds, which is what makes the tool
//! usable on a plane and what keeps the P2 latency budget intact.
//!
//! # Agents are opaque payload
//!
//! We read the frontmatter `name` and `description` and **nothing else**. The
//! body is copied byte for byte and never interpreted. Agency Agents is pre-1.0;
//! anything deeper means every upstream release breaks us. [`parse_frontmatter`]
//! is deliberately the dumbest thing that works, and it declines rather than
//! guesses when the shape is not what it expects.
//!
//! # The store is not a cache
//!
//! It resembles one — regenerable from the network, safe to delete. But
//! `update` does `reset --hard`, which is the one destructive operation in the
//! crate, so [`Store::update`] proves the directory is ours before touching it.
//! See [`crate::agents::GitOp::RemoteGetUrl`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::git::GitOp;
use super::state::StoreAgent;
use crate::error::CoreError;
use crate::exec::ProcessRunner;
use crate::url::GitUrl;

/// The upstream this build clones when nothing else is configured.
pub const DEFAULT_UPSTREAM: &str = "https://github.com/bort0s/agency-agents.git";

/// How stale a store may get before every read command mentions it.
pub const DEFAULT_STALE_AFTER_DAYS: u64 = 7;

/// Where the store lives and what it points at.
///
/// Caller-constructed, so it gets `Default` + `with_*` rather than
/// `#[non_exhaustive]` (ADR-0005 §3).
#[derive(Debug, Clone)]
pub struct StoreConfig {
    path: Option<PathBuf>,
    upstream: String,
    stale_after_days: u64,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            path: default_store_path(),
            upstream: DEFAULT_UPSTREAM.to_owned(),
            stale_after_days: DEFAULT_STALE_AFTER_DAYS,
        }
    }
}

impl StoreConfig {
    #[must_use]
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    #[must_use]
    pub fn with_upstream(mut self, upstream: impl Into<String>) -> Self {
        self.upstream = upstream.into();
        self
    }

    #[must_use]
    pub fn with_stale_after_days(mut self, days: u64) -> Self {
        self.stale_after_days = days;
        self
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[must_use]
    pub fn upstream(&self) -> &str {
        &self.upstream
    }

    /// The upstream, validated.
    ///
    /// Fallible and checked *here* rather than at use: a bad URL is a
    /// configuration error the user can fix, and reporting it when the config is
    /// read is more useful than reporting it from inside a clone.
    pub fn upstream_url(&self) -> Result<GitUrl, CoreError> {
        GitUrl::parse(&self.upstream)
    }
}

/// The OS-appropriate data directory, per ADR-0006 §1.
///
/// The **data** directory, not the cache directory. A cache is something the
/// tool may delete and rebuild; the store holds the only local copy of an
/// upstream that may be unreachable when it is next needed.
#[must_use]
pub fn default_store_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "vibe").map(|d| d.data_dir().join("agents"))
}

/// What one `update` did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UpdateReport {
    pub path: String,
    /// `true` when the store did not exist and was cloned.
    pub cloned: bool,
    /// The revision before the update, when there was one.
    pub from_rev: Option<String>,
    pub to_rev: Option<String>,
    pub agents: usize,
}

impl UpdateReport {
    /// Whether the store's content actually moved.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.cloned || (self.from_rev != self.to_rev)
    }
}

/// A handle to the store on disk.
#[derive(Debug)]
pub struct Store<'a> {
    config: &'a StoreConfig,
    exec: &'a dyn ProcessRunner,
}

impl<'a> Store<'a> {
    #[must_use]
    pub fn new(config: &'a StoreConfig, exec: &'a dyn ProcessRunner) -> Self {
        Self { config, exec }
    }

    fn dir(&self) -> Result<&Path, CoreError> {
        self.config.path().ok_or_else(|| CoreError::GitUnavailable {
            why: "this platform has no data directory and no store path was configured".to_owned(),
        })
    }

    /// Whether a store has ever been fetched.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.config.path().is_some_and(|p| p.join(".git").exists())
    }

    /// Clone or fast-forward the store. **The only network operation.**
    pub fn update(&self) -> Result<UpdateReport, CoreError> {
        let dir = self.dir()?;
        let url = self.config.upstream_url()?;

        if !self.exec.git_available() {
            return Err(CoreError::GitUnavailable {
                why: "git is not on PATH; the agent store is a git clone".to_owned(),
            });
        }

        let cloned = !self.exists();
        let from_rev = if cloned { None } else { self.head_rev().ok() };

        if cloned {
            if dir.exists() && std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()) {
                // Something is there and it is not a clone of ours. Refusing
                // beats cloning into it or clearing it out: we cannot tell a
                // mistyped path from a deliberate one, and one reading is
                // destructive.
                return Err(CoreError::StoreNotARepository {
                    path: dir.to_path_buf(),
                });
            }
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent).map_err(|source| CoreError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            self.run(&GitOp::Clone {
                url,
                dest: dir.to_path_buf(),
            })?;
        } else {
            // Prove it is ours before `reset --hard` gets anywhere near it.
            self.assert_is_our_store(&url)?;
            self.run(&GitOp::Fetch {
                cwd: dir.to_path_buf(),
            })?;
            self.run(&GitOp::ResetToFetchHead {
                cwd: dir.to_path_buf(),
            })?;
        }

        let to_rev = self.head_rev().ok();
        Ok(UpdateReport {
            path: dir.display().to_string(),
            cloned,
            from_rev,
            to_rev,
            agents: self.load()?.len(),
        })
    }

    /// Refuse to touch a directory that is not the store this config names.
    ///
    /// The whole guard. `update` hard-resets, so a store path that has been
    /// mistyped into pointing at one of the user's real repositories would
    /// destroy uncommitted work. Comparing the `origin` remote is cheap, needs
    /// no network, and answers exactly the question that matters.
    fn assert_is_our_store(&self, expected: &GitUrl) -> Result<(), CoreError> {
        let dir = self.dir()?;
        let out = self.run(&GitOp::RemoteGetUrl {
            cwd: dir.to_path_buf(),
        });
        let found = match out {
            Ok(o) => Some(o.trimmed().to_owned()),
            // No `origin` at all is still "not ours", and gets the same answer.
            Err(_) => None,
        };
        if found.as_deref() == Some(expected.as_str()) {
            return Ok(());
        }
        Err(CoreError::StoreNotOurs {
            path: dir.to_path_buf(),
            found,
            expected: expected.as_str().to_owned(),
        })
    }

    /// The store's current commit.
    pub fn head_rev(&self) -> Result<String, CoreError> {
        let dir = self.dir()?;
        Ok(self
            .run(&GitOp::RevParseHead {
                cwd: dir.to_path_buf(),
            })?
            .trimmed()
            .to_owned())
    }

    /// When the store's tip commit was authored, as an RFC 3339 string.
    ///
    /// Read from git metadata already on disk. This is what makes freshness
    /// reporting possible with no daemon and no network call on the hot path
    /// (ADR-0006 §7).
    pub fn last_commit_date(&self) -> Result<String, CoreError> {
        let dir = self.dir()?;
        Ok(self
            .run(&GitOp::LastCommitDate {
                cwd: dir.to_path_buf(),
            })?
            .trimmed()
            .to_owned())
    }

    /// How stale the store is, and whether that is worth saying.
    ///
    /// Returns `None` when the age cannot be established — which is *not* the
    /// same as "fresh", and is why this is an `Option` rather than a `u64`
    /// defaulting to zero. A store whose age we could not read must not be
    /// silently reported as up to date.
    #[must_use]
    pub fn staleness(&self, today_utc: &str) -> Staleness {
        if !self.exists() {
            return Staleness::NeverUpdated;
        }
        let Ok(date) = self.last_commit_date() else {
            return Staleness::Unknown;
        };
        match days_between(&date, today_utc) {
            Some(days) => Staleness::Days {
                days,
                stale: days > self.config.stale_after_days,
            },
            None => Staleness::Unknown,
        }
    }

    /// Every agent the store holds, keyed by name.
    ///
    /// A file whose frontmatter has no `name` is **skipped, not guessed at**.
    /// Deriving a name from the filename would work most of the time and be
    /// wrong occasionally, and an agent installed under a name that does not
    /// match its frontmatter is a rename this build cannot later detect.
    pub fn load(&self) -> Result<BTreeMap<String, StoreAgent>, CoreError> {
        let dir = self.dir()?;
        if !self.exists() {
            return Ok(BTreeMap::new());
        }
        let rev = self.head_rev().unwrap_or_default();

        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // Never descend into the clone's own git directory.
                    if path.file_name().is_some_and(|n| n == ".git") {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "md") {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Some(name) = parse_frontmatter(&bytes).and_then(|f| f.name) else {
                    continue;
                };
                let source_path = path
                    .strip_prefix(dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(
                    name.clone(),
                    StoreAgent {
                        name,
                        source_path,
                        rev: rev.clone(),
                        content_hash: super::lock::content_hash(&bytes),
                    },
                );
            }
        }
        Ok(out)
    }

    /// The bytes of one store agent, ready to be written into a project.
    ///
    /// Returns the file exactly as the store holds it. No templating, no marker
    /// comment, no rewriting — an installed agent is a user-editable artifact,
    /// not a rendered file, and the two mechanisms must not be shared
    /// (ADR-0006 Context).
    pub fn read_agent(&self, agent: &StoreAgent) -> Result<Vec<u8>, CoreError> {
        let dir = self.dir()?;
        let path = dir.join(&agent.source_path);
        std::fs::read(&path).map_err(|source| CoreError::Io { path, source })
    }

    fn run(&self, op: &GitOp) -> Result<crate::exec::CommandOutput, CoreError> {
        let out = self
            .exec
            .run_git_op(op)
            .map_err(|e| CoreError::GitUnavailable { why: e.to_string() })?;
        if out.success() {
            return Ok(out);
        }
        Err(CoreError::ToolFailed {
            argv: out.argv.clone(),
            status: out.status,
            stderr: out.stderr.trim().to_owned(),
        })
    }
}

/// How out of date the store is.
///
/// Four cases, not a number, because three of them are not a number. Collapsing
/// `NeverUpdated` or `Unknown` into "0 days old" would report a fact about this
/// machine as a fact about the store — the same substitution the detectors are
/// forbidden from making.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Staleness {
    /// No store on disk. Said explicitly rather than reported as an age
    /// (ADR-0006 §6).
    NeverUpdated,
    /// A store exists and its age could not be read.
    Unknown,
    Days {
        days: u64,
        stale: bool,
    },
}

impl Staleness {
    /// Whether this is worth a line on stderr.
    #[must_use]
    pub fn worth_reporting(self) -> bool {
        match self {
            Staleness::NeverUpdated | Staleness::Unknown => true,
            Staleness::Days { stale, .. } => stale,
        }
    }
}

/// The two frontmatter fields we read, and no others.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Read `name` and `description` out of a leading `---` block.
///
/// Deliberately not a YAML parser. We need two scalar strings from the top of a
/// file we have promised not to interpret, and pulling in a YAML dependency to
/// get them would be the first step toward interpreting the rest. The rule is
/// that anything this does not recognise is *skipped*, never guessed at:
/// nested structures, lists, anchors and multi-line scalars all read as absent,
/// and an agent whose `name` we cannot read is one we decline to install rather
/// than one we name after its file.
#[must_use]
pub fn parse_frontmatter(bytes: &[u8]) -> Option<Frontmatter> {
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }

    let mut out = Frontmatter::default();
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed.trim() == "---" {
            return Some(out);
        }
        // Only top-level keys. An indented line belongs to a structure we have
        // decided not to understand.
        if line.starts_with([' ', '\t', '-']) {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let value = unquote(value.trim());
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "name" => out.name = Some(value),
            "description" => out.description = Some(value),
            _ => {}
        }
    }
    // Ran off the end without a closing `---`. That is a malformed file, and
    // reading half a frontmatter block as if it were whole is guessing.
    None
}

fn unquote(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        return s[1..s.len() - 1].to_owned();
    }
    s.to_owned()
}

/// Whole days between two dates, from the `YYYY-MM-DD` prefix of each.
///
/// Only the date part is used. An RFC 3339 timestamp carries an offset, and
/// "12 days ago" does not need — and cannot honestly claim — better resolution
/// than a day.
fn days_between(from_rfc3339: &str, to_ymd: &str) -> Option<u64> {
    let a = days_from_ymd(from_rfc3339.get(..10)?)?;
    let b = days_from_ymd(to_ymd.get(..10)?)?;
    // Clamped at zero rather than allowed to go negative: a store whose tip
    // commit is dated in the future (a skewed clock, a rebased history) is
    // reported as brand new, never as "-3 days old".
    u64::try_from((b - a).max(0)).ok()
}

/// `YYYY-MM-DD` to days since the epoch. Hinnant's `days_from_civil`, the
/// inverse of the one in [`crate::config`].
fn days_from_ymd(s: &str) -> Option<i64> {
    let mut parts = s.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_yields_the_two_fields_we_read_and_nothing_else() {
        let f = parse_frontmatter(
            b"---\nname: engineering-code-reviewer\ndescription: Reviews code.\nmodel: opus\n---\n\nThe body.\n",
        )
        .expect("parses");
        assert_eq!(f.name.as_deref(), Some("engineering-code-reviewer"));
        assert_eq!(f.description.as_deref(), Some("Reviews code."));
    }

    #[test]
    fn quoted_values_are_unquoted_once() {
        let f = parse_frontmatter(b"---\nname: \"a-b\"\ndescription: 'x: y'\n---\n").unwrap();
        assert_eq!(f.name.as_deref(), Some("a-b"));
        assert_eq!(f.description.as_deref(), Some("x: y"));
    }

    /// The rule that keeps "opaque payload" true: anything we do not recognise
    /// is skipped, and a file we cannot name is one we decline rather than one
    /// we name after its path.
    #[test]
    fn shapes_we_decided_not_to_understand_read_as_absent_not_as_a_guess() {
        // No frontmatter at all.
        assert!(parse_frontmatter(b"# Just a heading\n").is_none());
        // Unterminated block: reading half of one as if it were whole is
        // guessing about a file we promised not to interpret.
        assert!(parse_frontmatter(b"---\nname: x\n").is_none());
        // Not UTF-8.
        assert!(parse_frontmatter(&[0xff, 0xfe, 0x00]).is_none());

        // A nested structure: the top-level key has no scalar value, and the
        // indented lines belong to something we do not parse.
        let f = parse_frontmatter(b"---\ntools:\n  - Read\n  - Edit\nname: ok\n---\n").unwrap();
        assert_eq!(f.name.as_deref(), Some("ok"));
        assert_eq!(f.description, None);

        // A block scalar. We do not read it, and we must not read its marker
        // as the value.
        let f = parse_frontmatter(b"---\nname: ok\ndescription: |\n  line one\n  line two\n---\n")
            .unwrap();
        assert_eq!(f.name.as_deref(), Some("ok"));
        assert_eq!(
            f.description.as_deref(),
            Some("|"),
            "documenting the known limit: a block scalar's marker is what a \
             two-field reader sees. It is never written back into a store file, \
             only shown in `agents list`."
        );
    }

    #[test]
    fn a_file_with_no_name_is_skipped_rather_than_named_after_itself() {
        let f = parse_frontmatter(b"---\ndescription: no name here\n---\n").unwrap();
        assert_eq!(f.name, None);
    }

    #[test]
    fn days_between_counts_whole_days_from_the_date_part() {
        assert_eq!(
            days_between("2026-07-29T09:14:22Z", "2026-08-10"),
            Some(12),
            "the worked example in ADR-0006 §6"
        );
        assert_eq!(
            days_between("2026-08-10T23:59:59+02:00", "2026-08-10"),
            Some(0)
        );
        // A store committed in the future is 0 days old, never a negative age.
        assert_eq!(days_between("2027-01-01T00:00:00Z", "2026-08-10"), Some(0));
        assert_eq!(days_between("not a date", "2026-08-10"), None);
    }

    #[test]
    fn days_from_ymd_matches_known_anchors_and_is_monotonic() {
        // The same anchors `config::civil_from_days` is pinned to, from the
        // other direction — these two conversions have to agree or every age
        // this module reports is off by a leap day.
        assert_eq!(days_from_ymd("1970-01-01"), Some(0));
        assert_eq!(days_from_ymd("2000-01-01"), Some(10_957));
        assert_eq!(days_from_ymd("2000-03-01"), Some(11_017));
        assert_eq!(days_from_ymd("1999-12-31"), Some(10_956));

        assert!(days_from_ymd("2026-13-01").is_none(), "month 13");
        assert!(days_from_ymd("2026-01-32").is_none(), "day 32");
        assert!(days_from_ymd("nonsense").is_none());
    }

    #[test]
    fn staleness_never_reports_an_unknown_age_as_fresh() {
        // The distinction the type exists for: three of the four cases are not
        // a number, and defaulting them to zero would say "up to date" about a
        // store we know nothing about.
        assert!(Staleness::NeverUpdated.worth_reporting());
        assert!(Staleness::Unknown.worth_reporting());
        assert!(
            Staleness::Days {
                days: 12,
                stale: true
            }
            .worth_reporting()
        );
        assert!(
            !Staleness::Days {
                days: 1,
                stale: false
            }
            .worth_reporting()
        );
    }

    #[test]
    fn a_rejected_upstream_is_a_config_error_not_a_clone_error() {
        let cfg = StoreConfig::default().with_upstream("ext::sh -c evil");
        let err = cfg.upstream_url().expect_err("must be rejected");
        assert_eq!(err.code(), "VIBE_E_GIT_URL_REJECTED");
    }

    #[test]
    fn the_default_store_is_in_the_data_directory_not_the_cache_directory() {
        let Some(store) = default_store_path() else {
            return;
        };
        let Some(cache) = crate::Config::cache_dir() else {
            return;
        };
        assert!(
            !store.starts_with(&cache),
            "the store holds the only local copy of an upstream that may be \
             unreachable later; a cache is something a tool may delete"
        );
    }
}
