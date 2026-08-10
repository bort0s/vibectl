//! The ten agent states, and the one function that decides them.
//!
//! Every command's behaviour is a function of the state, so the state is
//! computed in exactly one place. Two distinctions here do most of the work:
//!
//! - **`PresentUnowned` is not `Modified`.** A file we did not install is not a
//!   modified copy of ours. It might be hand-written, or installed by another
//!   tool, and calling it modified asserts a relationship that does not exist —
//!   which is how the next `update` overwrites someone else's work.
//! - **`Unverifiable` is not `Modified` either.** When the lockfile is corrupt
//!   or the hash algorithm is unknown, we cannot tell. Reporting "cannot tell"
//!   as "differs" is the same substitution as reporting a missing `git` as
//!   "this project has no remote".
//!
//! ADR-0006 §5.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;

use super::lock::{LockLoad, LockedAgent, content_hash, hash_is_verifiable};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentState {
    /// Ours, on disk, unmodified.
    Installed,
    /// Ours, on disk, and edited since we wrote it. Editing an installed agent
    /// is the *normal* reason to have installed one, so this never overwrites
    /// without an explicit flag.
    Modified,
    /// Ours according to the lockfile, but the file is gone.
    Missing,
    /// A file we did not install, whose name matches a store agent. Not
    /// adopted, not deleted, not modified — reported so the user decides.
    PresentUnowned,
    /// Ours, still on disk, but the store no longer has it. The local file is
    /// **never** deleted: upstream removing something is not the user's
    /// instruction to delete their copy.
    Orphaned,
    /// As `Orphaned`, but the same content exists in the store under a new
    /// name.
    RenamedUpstream,
    /// Declared in the manifest, not installed. This is what `sync` is for.
    Declared,
    /// Installed by us, but the manifest does not declare it. Left alone —
    /// usually it means someone added it and did not commit the manifest.
    Undeclared,
    /// The store does not have a declared agent, and we cannot say why.
    NotInStore,
    /// Ownership or content cannot be established: a corrupt lockfile, an
    /// unsupported lock version, or a hash from an algorithm this build does
    /// not know.
    Unverifiable,
}

impl AgentState {
    /// Whether a command may modify or delete this agent's file.
    ///
    /// The single place the ownership rule is enforced. Everything that writes
    /// asks this rather than re-deriving it.
    #[must_use]
    pub fn is_ours_to_touch(self) -> bool {
        matches!(
            self,
            AgentState::Installed
                | AgentState::Modified
                | AgentState::Missing
                | AgentState::Orphaned
                | AgentState::RenamedUpstream
                | AgentState::Undeclared
        )
    }

    /// Whether this state blocks an overwrite without an explicit flag.
    #[must_use]
    pub fn needs_force_to_overwrite(self) -> bool {
        matches!(self, AgentState::Modified | AgentState::Unverifiable)
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AgentState::Installed => "ok",
            AgentState::Modified => "modified",
            AgentState::Missing => "missing",
            AgentState::PresentUnowned => "unowned",
            AgentState::Orphaned => "orphaned",
            AgentState::RenamedUpstream => "orphaned (renamed upstream)",
            AgentState::Declared => "declared",
            AgentState::Undeclared => "undeclared",
            AgentState::NotInStore => "not in store",
            AgentState::Unverifiable => "unverifiable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct InstalledAgent {
    pub name: String,
    pub state: AgentState,
    /// Project-relative path, when there is a file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The store revision this copy came from, when we recorded one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_rev: Option<String>,
    /// True when the store has a newer revision than the one recorded.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub outdated: bool,
    /// For `RenamedUpstream`, the name it now has in the store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct AgentReport {
    pub agents: Vec<InstalledAgent>,
    /// True when ownership could not be established at all. Every write must
    /// refuse while this holds.
    pub ownership_known: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_note: Option<String>,
}

impl AgentReport {
    #[must_use]
    pub fn in_state(&self, state: AgentState) -> Vec<&InstalledAgent> {
        self.agents.iter().filter(|a| a.state == state).collect()
    }

    #[must_use]
    pub fn any(&self, state: AgentState) -> bool {
        self.agents.iter().any(|a| a.state == state)
    }
}

/// What the store knows about one agent, as far as state computation cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreAgent {
    pub name: String,
    pub source_path: String,
    /// The store's current revision.
    pub rev: String,
    /// Hash of the store's copy, for detecting an upstream rename by content.
    pub content_hash: String,
}

/// Decide the state of every agent this project knows about.
///
/// Inputs are deliberately plain data: the lockfile as loaded, what the store
/// holds, what the manifest declares, and a way to read the installed file.
/// That makes every state in the table reachable from a unit test without a
/// filesystem, a git repository, or a store.
#[must_use]
pub fn compute(
    lock_load: &LockLoad,
    store: &BTreeMap<String, StoreAgent>,
    declared: &BTreeSet<String>,
    project_dir: &Path,
) -> AgentReport {
    let ownership_known = lock_load.ownership_known();
    let lock_note = lock_load.note();
    let locked: BTreeMap<String, LockedAgent> = lock_load
        .usable()
        .map(|l| l.agents.clone())
        .unwrap_or_default();

    let mut agents: Vec<InstalledAgent> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    // Everything we believe we installed.
    for (name, entry) in &locked {
        seen.insert(name.clone());
        let file = project_dir.join(&entry.path);
        let on_disk = std::fs::read(&file).ok();

        let store_entry = store.get(name);
        let state = if !ownership_known {
            AgentState::Unverifiable
        } else if on_disk.is_none() {
            AgentState::Missing
        } else if !hash_is_verifiable(&entry.content_hash) {
            // A hash we cannot recompute. "Cannot tell" is not "differs".
            AgentState::Unverifiable
        } else if on_disk.as_deref().map(content_hash).as_deref() != Some(&entry.content_hash) {
            AgentState::Modified
        } else if store_entry.is_none() {
            // Gone from the store. Look for the same content under another
            // name before calling it a deletion.
            match store
                .values()
                .find(|s| s.content_hash == entry.content_hash)
            {
                Some(_) => AgentState::RenamedUpstream,
                None => AgentState::Orphaned,
            }
        } else if !declared.contains(name) {
            AgentState::Undeclared
        } else {
            AgentState::Installed
        };

        let renamed_to = if state == AgentState::RenamedUpstream {
            store
                .values()
                .find(|s| s.content_hash == entry.content_hash)
                .map(|s| s.name.clone())
        } else {
            None
        };

        agents.push(InstalledAgent {
            name: name.clone(),
            state,
            path: Some(entry.path.clone()),
            source_rev: Some(entry.source_rev.clone()),
            outdated: store_entry.is_some_and(|s| s.rev != entry.source_rev)
                && state == AgentState::Installed,
            renamed_to,
        });
    }

    // Declared but not in the lockfile: either waiting to be installed, or
    // present as somebody else's file, or absent from the store entirely.
    for name in declared {
        if seen.contains(name) {
            continue;
        }
        seen.insert(name.clone());
        let path = format!(".claude/agents/{name}.md");
        let exists = project_dir.join(&path).exists();

        let state = if !ownership_known {
            AgentState::Unverifiable
        } else if exists {
            // A file we did not install. Not adopted, not overwritten.
            AgentState::PresentUnowned
        } else if store.contains_key(name) {
            AgentState::Declared
        } else {
            // We cannot distinguish a stale store from an upstream deletion
            // from a typo, and we do not guess. The caller qualifies this with
            // the store's age — a stale store is a fact about this machine, not
            // a property of the project.
            AgentState::NotInStore
        };

        agents.push(InstalledAgent {
            name: name.clone(),
            state,
            path: exists.then_some(path),
            source_rev: None,
            outdated: false,
            renamed_to: None,
        });
    }

    agents.sort_by(|a, b| a.name.cmp(&b.name));
    AgentReport {
        agents,
        ownership_known,
        lock_note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::lock::{LOCK_VERSION, Lockfile};

    fn store_agent(name: &str, rev: &str, body: &[u8]) -> StoreAgent {
        StoreAgent {
            name: name.to_owned(),
            source_path: format!("{name}.md"),
            rev: rev.to_owned(),
            content_hash: content_hash(body),
        }
    }

    fn store(entries: Vec<StoreAgent>) -> BTreeMap<String, StoreAgent> {
        entries.into_iter().map(|s| (s.name.clone(), s)).collect()
    }

    fn locked(name: &str, rev: &str, body: &[u8]) -> LockedAgent {
        LockedAgent {
            name: name.to_owned(),
            path: format!(".claude/agents/{name}.md"),
            content_hash: content_hash(body),
            source_rev: rev.to_owned(),
            source_path: format!("{name}.md"),
            installed_at: "2026-08-10T00:00:00Z".to_owned(),
        }
    }

    fn lockfile(entries: Vec<LockedAgent>) -> LockLoad {
        LockLoad::Loaded(Lockfile {
            version: LOCK_VERSION,
            agents: entries.into_iter().map(|a| (a.name.clone(), a)).collect(),
        })
    }

    fn declared(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Write an installed agent file into a project directory.
    fn install(dir: &Path, name: &str, body: &[u8]) {
        let p = dir.join(".claude/agents").join(format!("{name}.md"));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn state_of(report: &AgentReport, name: &str) -> AgentState {
        report
            .agents
            .iter()
            .find(|a| a.name == name)
            .unwrap_or_else(|| panic!("{name} not in report"))
            .state
    }

    #[test]
    fn an_unmodified_installed_agent_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "a", b"body");
        let report = compute(
            &lockfile(vec![locked("a", "r1", b"body")]),
            &store(vec![store_agent("a", "r1", b"body")]),
            &declared(&["a"]),
            dir.path(),
        );
        assert_eq!(state_of(&report, "a"), AgentState::Installed);
        assert!(!report.agents[0].outdated);
    }

    #[test]
    fn a_store_at_a_newer_revision_marks_it_outdated_without_changing_state() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "a", b"body");
        let report = compute(
            &lockfile(vec![locked("a", "r1", b"body")]),
            &store(vec![store_agent("a", "r2", b"body")]),
            &declared(&["a"]),
            dir.path(),
        );
        assert_eq!(state_of(&report, "a"), AgentState::Installed);
        assert!(report.agents[0].outdated);
    }

    #[test]
    fn an_edited_agent_is_modified_and_needs_force() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "a", b"the user changed this");
        let report = compute(
            &lockfile(vec![locked("a", "r1", b"body")]),
            &store(vec![store_agent("a", "r1", b"body")]),
            &declared(&["a"]),
            dir.path(),
        );
        assert_eq!(state_of(&report, "a"), AgentState::Modified);
        assert!(AgentState::Modified.needs_force_to_overwrite());
        assert!(AgentState::Modified.is_ours_to_touch(), "still ours");
    }

    #[test]
    fn a_file_we_did_not_install_is_unowned_and_never_ours_to_touch() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "a", b"someone else wrote this");
        // No lock entry: we never installed it.
        let report = compute(
            &LockLoad::Absent,
            &store(vec![store_agent("a", "r1", b"body")]),
            &declared(&["a"]),
            dir.path(),
        );
        assert_eq!(state_of(&report, "a"), AgentState::PresentUnowned);
        assert!(
            !AgentState::PresentUnowned.is_ours_to_touch(),
            "adopting a file we did not write is how the next update destroys work"
        );
    }

    #[test]
    fn a_missing_file_with_a_lock_entry_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let report = compute(
            &lockfile(vec![locked("a", "r1", b"body")]),
            &store(vec![store_agent("a", "r1", b"body")]),
            &declared(&["a"]),
            dir.path(),
        );
        assert_eq!(state_of(&report, "a"), AgentState::Missing);
    }

    #[test]
    fn an_agent_gone_from_the_store_is_orphaned_and_stays_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "a", b"body");
        let report = compute(
            &lockfile(vec![locked("a", "r1", b"body")]),
            &store(vec![]),
            &declared(&["a"]),
            dir.path(),
        );
        assert_eq!(state_of(&report, "a"), AgentState::Orphaned);
        // Upstream deleting something is not the user's instruction to delete
        // their copy. The file is still on disk.
        assert!(dir.path().join(".claude/agents/a.md").exists());
    }

    #[test]
    fn the_same_content_under_a_new_name_is_a_rename_not_a_deletion() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "a", b"body");
        let report = compute(
            &lockfile(vec![locked("a", "r1", b"body")]),
            &store(vec![store_agent("b", "r2", b"body")]),
            &declared(&["a"]),
            dir.path(),
        );
        assert_eq!(state_of(&report, "a"), AgentState::RenamedUpstream);
        assert_eq!(report.agents[0].renamed_to.as_deref(), Some("b"));
    }

    #[test]
    fn declared_but_not_installed_is_what_sync_is_for() {
        let dir = tempfile::tempdir().unwrap();
        let report = compute(
            &LockLoad::Absent,
            &store(vec![store_agent("a", "r1", b"body")]),
            &declared(&["a"]),
            dir.path(),
        );
        assert_eq!(state_of(&report, "a"), AgentState::Declared);
    }

    #[test]
    fn installed_but_undeclared_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "a", b"body");
        let report = compute(
            &lockfile(vec![locked("a", "r1", b"body")]),
            &store(vec![store_agent("a", "r1", b"body")]),
            &declared(&[]),
            dir.path(),
        );
        assert_eq!(state_of(&report, "a"), AgentState::Undeclared);
    }

    #[test]
    fn a_declared_agent_the_store_lacks_is_not_in_store() {
        let dir = tempfile::tempdir().unwrap();
        let report = compute(
            &LockLoad::Absent,
            &store(vec![]),
            &declared(&["typo"]),
            dir.path(),
        );
        assert_eq!(state_of(&report, "typo"), AgentState::NotInStore);
    }

    /// The distinction the whole lockfile design rests on.
    #[test]
    fn a_corrupt_lockfile_makes_everything_unverifiable_not_unowned() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "a", b"body");
        let report = compute(
            &LockLoad::Corrupt {
                why: "bad toml".to_owned(),
            },
            &store(vec![store_agent("a", "r1", b"body")]),
            &declared(&["a"]),
            dir.path(),
        );

        assert!(!report.ownership_known);
        assert_eq!(state_of(&report, "a"), AgentState::Unverifiable);
        // Not PresentUnowned: that would be a positive claim that the file is
        // someone else's, when the truth is that we cannot tell.
        assert_ne!(state_of(&report, "a"), AgentState::PresentUnowned);
        assert!(AgentState::Unverifiable.needs_force_to_overwrite());
        assert!(report.lock_note.is_some());
    }

    #[test]
    fn a_hash_from_an_unknown_algorithm_is_unverifiable_not_modified() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "a", b"body");
        let mut entry = locked("a", "r1", b"body");
        entry.content_hash = "sha256:deadbeef".to_owned();

        let report = compute(
            &lockfile(vec![entry]),
            &store(vec![store_agent("a", "r1", b"body")]),
            &declared(&["a"]),
            dir.path(),
        );
        assert_eq!(
            state_of(&report, "a"),
            AgentState::Unverifiable,
            "reporting this as Modified would offer to overwrite untouched work \
             after an algorithm change"
        );
    }

    /// ADR-0006 §4a: a crash partway through `update` is N independent states.
    #[test]
    fn a_half_finished_update_is_per_agent_states_not_a_corrupt_one() {
        let dir = tempfile::tempdir().unwrap();
        // `a` was rewritten and its entry advanced; `b` was not reached; `c`
        // was written but crashed before its entry was updated.
        install(dir.path(), "a", b"new");
        install(dir.path(), "b", b"old");
        install(dir.path(), "c", b"new");

        let report = compute(
            &lockfile(vec![
                locked("a", "r2", b"new"),
                locked("b", "r1", b"old"),
                locked("c", "r1", b"old"),
            ]),
            &store(vec![
                store_agent("a", "r2", b"new"),
                store_agent("b", "r2", b"new"),
                store_agent("c", "r2", b"new"),
            ]),
            &declared(&["a", "b", "c"]),
            dir.path(),
        );

        assert_eq!(state_of(&report, "a"), AgentState::Installed, "done");
        assert_eq!(state_of(&report, "b"), AgentState::Installed, "not reached");
        assert!(
            report
                .agents
                .iter()
                .find(|x| x.name == "b")
                .unwrap()
                .outdated,
            "and reported as outdated"
        );
        // The one in flight: from the lockfile alone, "we crashed writing this"
        // and "the user edited it" are the same observation, and erring toward
        // not destroying an edit is the honest reading.
        assert_eq!(state_of(&report, "c"), AgentState::Modified, "in flight");
    }

    #[test]
    fn the_ownership_rule_is_answered_in_one_place() {
        // Nothing not installed by us may be touched.
        for state in [
            AgentState::PresentUnowned,
            AgentState::Unverifiable,
            AgentState::NotInStore,
        ] {
            assert!(!state.is_ours_to_touch(), "{state:?}");
        }
        for state in [
            AgentState::Installed,
            AgentState::Modified,
            AgentState::Orphaned,
        ] {
            assert!(state.is_ours_to_touch(), "{state:?}");
        }
    }
}
