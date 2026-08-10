//! The cache is regenerable and never authoritative, so prove it.
//!
//! Every case here damages the cache in a different way and asserts the same
//! two things: the answer is still **correct**, and the user is not made to
//! deal with it. A cache that can produce a wrong answer is worse than no cache
//! at all, because `list` is the command people trust without checking.
//!
//! `rm -rf` the cache is a **supported recovery path**, tested as one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use vibe_core::cache::{Cache, CacheFile, Witness, cache_file_name};
use vibe_core::{Config, NoRunner, NullReporter, Query, Registry, ScanRequest};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, contents).expect("write");
}

struct Fixture {
    _dir: tempfile::TempDir,
    projects: PathBuf,
    cache_file: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let projects = dir.path().join("projects");
    let cache_dir = dir.path().join("cache");
    std::fs::create_dir_all(&cache_dir).expect("mkdir");

    for (name, runtime) in [("alpha", "node"), ("beta", "rust")] {
        let root = projects.join(name);
        write(
            &root,
            ".vibe/project.toml",
            &format!(
                "schema_version = \"1.0\"\n\n[project]\nname = \"{name}\"\nstatus = \"active\"\n"
            ),
        );
        match runtime {
            "node" => write(&root, "package.json", r#"{"name":"alpha"}"#),
            _ => write(&root, "Cargo.toml", "[package]\nname = \"beta\"\n"),
        }
    }

    Fixture {
        _dir: dir,
        projects,
        cache_file: cache_dir.join(cache_file_name()),
    }
}

impl Fixture {
    fn registry(&self) -> Registry {
        Registry::open(Config::default())
            .with_runner(Arc::new(NoRunner))
            .with_cache_path(Some(self.cache_file.clone()))
    }

    fn list(&self) -> vibe_core::ListReport {
        self.registry().list(
            &ScanRequest::new(&self.projects),
            &Query::default(),
            &NullReporter,
        )
    }

    fn names(&self) -> Vec<String> {
        let mut n: Vec<String> = self
            .list()
            .projects
            .iter()
            .map(|p| p.name.clone())
            .collect();
        n.sort();
        n
    }
}

#[test]
fn a_first_run_builds_the_cache_and_a_second_uses_it() {
    let f = fixture();

    let first = f.list();
    assert_eq!(first.projects.len(), 2);
    assert_eq!(first.from_cache, 0, "nothing to read on a first run");
    assert!(first.cache_note.is_none(), "a first run is not an event");
    assert!(f.cache_file.exists(), "the cache should have been written");

    let second = f.list();
    assert_eq!(second.from_cache, 2, "the second run should hit the cache");
    assert_eq!(
        first.projects.iter().map(|p| &p.name).collect::<Vec<_>>(),
        second.projects.iter().map(|p| &p.name).collect::<Vec<_>>(),
        "a cached answer must equal the fresh one"
    );
}

#[test]
fn deleting_the_cache_is_a_supported_recovery_path() {
    let f = fixture();
    let before = f.names();
    assert!(f.list().projects.len() == 2);

    std::fs::remove_file(&f.cache_file).expect("rm");

    let after = f.list();
    assert_eq!(after.from_cache, 0, "there is nothing to read");
    assert!(
        after.cache_note.is_none(),
        "deleting the cache is not an error and must not be reported as one"
    );
    assert_eq!(f.names(), before, "the answer is unchanged");
    assert!(f.cache_file.exists(), "and the cache is rebuilt");
}

#[test]
fn a_corrupt_cache_degrades_to_a_rescan_and_still_answers_correctly() {
    let f = fixture();
    let before = f.names();
    f.list();

    std::fs::write(&f.cache_file, "{ this is not json at all").expect("write");

    let after = f.list();
    assert_eq!(after.from_cache, 0);
    assert!(
        after.cache_note.is_some_and(|n| n.contains("rebuilding")),
        "the user is told, once, in a note - not an error"
    );
    assert_eq!(
        f.names(),
        before,
        "a corrupt cache must not change the answer"
    );
}

#[test]
fn a_cache_from_another_version_is_ignored_and_left_alone() {
    let f = fixture();
    let before = f.names();
    std::fs::write(&f.cache_file, r#"{"version":99,"entries":{}}"#).expect("write");

    let after = f.list();
    assert!(after.cache_note.is_some_and(|n| n.contains("version 99")));
    assert_eq!(f.names(), before);
}

#[test]
fn a_stale_entry_loses_to_the_manifest_on_disk() {
    let f = fixture();
    f.list();
    assert_eq!(f.list().from_cache, 2, "warm");

    // Edit a manifest behind the cache's back, the way a person or a merge
    // would. The recorded witness no longer matches, so the row is re-read.
    let manifest = f.projects.join("alpha/.vibe/project.toml");
    let text = std::fs::read_to_string(&manifest).expect("read");
    std::fs::write(&manifest, text.replace("active", "shipped")).expect("write");

    let after = f.list();
    assert_eq!(after.from_cache, 1, "only beta is still valid");
    let alpha = after
        .projects
        .iter()
        .find(|p| p.name == "alpha")
        .expect("alpha");
    assert_eq!(
        alpha.status, "shipped",
        "disk wins unconditionally; there is no tie-break"
    );
    assert!(!alpha.from_cache);
}

/// A cache entry that flatly disagrees with disk, rather than merely being out
/// of date. Only the witness protects against this, so this is the test that
/// says the witness is load-bearing.
#[test]
fn a_cache_entry_that_lies_is_overridden_by_disk() {
    let f = fixture();
    f.list();

    let mut file: CacheFile =
        serde_json::from_str(&std::fs::read_to_string(&f.cache_file).expect("read"))
            .expect("parse");
    let key = file
        .entries
        .keys()
        .find(|k| k.ends_with("alpha"))
        .expect("alpha entry")
        .clone();
    let entry = file.entries.get_mut(&key).expect("entry");
    entry.name = "NOT-THE-REAL-NAME".to_owned();
    entry.status = "dead".to_owned();
    // Invalidate the witness, which is what a real edit would also do.
    entry.witness = Witness {
        manifest_len: Some(1),
        manifest_mtime_ms: Some(1),
    };
    Cache::at(Some(f.cache_file.clone())).save(&file);

    let after = f.list();
    assert!(
        after.projects.iter().any(|p| p.name == "alpha"),
        "the manifest on disk is the truth: {:?}",
        after.projects.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    assert!(
        !after.projects.iter().any(|p| p.name == "NOT-THE-REAL-NAME"),
        "a lying cache entry reached the output"
    );
}

#[test]
fn entries_for_projects_that_no_longer_exist_do_not_resurrect_them() {
    let f = fixture();
    f.list();
    assert_eq!(f.names(), vec!["alpha", "beta"]);

    std::fs::remove_dir_all(f.projects.join("beta")).expect("rm");

    let after = f.list();
    assert_eq!(
        after.projects.len(),
        1,
        "a deleted project must not be listed from a cache entry: {:?}",
        after.projects.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    assert_eq!(after.projects[0].name, "alpha");

    // The ghost entry is *retained* in the file on purpose (ADR-0004): evicting
    // on absence would let a scan of one root delete the registry's memory of
    // another. It is harmless because listing is driven by the walk, not by the
    // cache's key set — which is exactly what this test pins down.
    let file: CacheFile =
        serde_json::from_str(&std::fs::read_to_string(&f.cache_file).expect("read"))
            .expect("parse");
    assert!(
        file.entries.keys().any(|k| k.ends_with("beta")),
        "the ghost entry is expected to remain"
    );
}

#[test]
fn an_unwritable_cache_location_does_not_break_anything() {
    let f = fixture();
    let registry = Registry::open(Config::default())
        .with_runner(Arc::new(NoRunner))
        .with_cache_path(None);

    let report = registry.list(
        &ScanRequest::new(&f.projects),
        &Query::default(),
        &NullReporter,
    );
    assert_eq!(report.projects.len(), 2, "everything still works");
    assert_eq!(report.from_cache, 0);
    assert!(report.cache_note.is_some_and(|n| n.contains("not caching")));
}

#[test]
fn show_never_serves_a_cached_answer() {
    let f = fixture();
    f.list();

    // Change the manifest without touching the cache.
    let manifest = f.projects.join("alpha/.vibe/project.toml");
    let text = std::fs::read_to_string(&manifest).expect("read");
    std::fs::write(&manifest, text.replace("active", "paused")).expect("write");

    let view = f
        .registry()
        .show(&f.projects.join("alpha"))
        .expect("show reads the manifest");
    assert_eq!(
        view.manifest.project.status.to_string(),
        "paused",
        "show must re-read, always"
    );
}
