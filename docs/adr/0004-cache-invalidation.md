# ADR-0004: Global Cache — Contents, Staleness, and Invalidation

## Status

Accepted (2026-08-10)

## Context

The global cache lives at the `directories`-provided path, is fully regenerable, and is **never authoritative**. That is fixed. What is not fixed is what "never authoritative" actually costs us at each decision point, and that is where this gets interesting.

The cache exists because `vibe list` must be instant. Re-walking 50 project directories takes ~2 seconds; a user who runs `vibe list` twenty times a day will not tolerate that, and the whole premise of the product — a registry you actually consult — dies if the registry is slow.

Four things must be decided:

1. What goes in it (and, more importantly, what must *not*, because putting the wrong thing in a regenerable file quietly makes it authoritative).
2. How staleness is detected, given that mtime lies, content hashing costs, and git HEAD misses dirty trees.
3. What happens when the cache and a manifest disagree.
4. What happens when it is corrupt, or from a build that does not exist yet.

The failure mode to avoid: a cache that is *technically* regenerable but that the user would be sad to lose. That is authority by accident.

## Decision

### 1. Split: config holds intent, cache holds only derived data

This is the first and most important decision, and it is easy to get wrong.

- **`config_dir()/config.toml`** — the list of scanned roots, plus preferences. **This is not cache.** Which directories the user asked us to index is *user intent*, not derived data. If the roots lived in the cache, deleting the cache would make `vibe list` forget where the user's projects are, and the cache would be authoritative for something after all.
- **`cache_dir()/index.v1.json`** — everything derived. Deleting it costs time, never information.

The test for whether something belongs in the cache: *can `vibe scan` reconstruct it from disk alone?* If no, it is config.

### 2. Format and contents

One JSON file. Not TOML (nobody hand-edits this, and TOML buys nothing), not a binary format (undebuggable, and version-fragile in a file we want to be trivially discardable), not SQLite (not in the fixed dependency set, and unnecessary at registry scale — hundreds of entries, not millions).

```jsonc
{
  "cache_version": 1,
  "vibectl_version": "0.1.0",     // diagnostics only, never a validity input
  "detector_set_version": 3,      // bumped when detector logic changes
  "generated_at": "2026-08-10T09:23:11Z",
  "entries": [{
    "project_root": "C:/Users/r/projects/macroring",
    "manifest_path": "C:/Users/r/projects/macroring/.vibe/project.toml",

    // Denormalized so `vibe list` renders without opening anything.
    "name": "macroring", "status": "active", "archived": false,
    "description": "...", "schema_version": 1,
    "stack_summary": "node@22 · react@19 · vite",
    "repo_remote": "github.com/user/macroring",
    "deploy_url": "https://macroring.vercel.app",
    "git_last_commit": "2026-08-04T18:22:03Z",

    // Staleness witnesses (§3).
    "manifest_mtime": "2026-08-04T18:20:00.123Z",
    "manifest_len": 812,
    "manifest_digest": "blake3:9f2c…",
    "detect_input_digest": "blake3:41ab…",
    "git_head_oid": "a1b2c3…",

    // Provenance for ADR-0003 §6: the exact values we last wrote.
    "written_fields": { "stack.runtime": "node@22", "deploy.url": "https://…" },

    "missing": false,
    "last_seen": "2026-08-10T09:23:11Z"
  }]
}
```

**The cache stores no secrets.** `deploy.env_required` holds variable *names*; `.env` is never read at all (ADR-0003 §2). Deploy URLs are public by nature. Nothing in `cache_dir()` needs protecting beyond ordinary file permissions, which is deliberate — a cache directory is not a place to defend a secret.

### 3. Staleness: layered witnesses, cheapest first, and separate keys for two different things

Manifest data and detection data go stale for different reasons and are invalidated independently.

**Manifest freshness** — `manifest_mtime` + `manifest_len`, then `manifest_digest`:

- The cheap stat (mtime + len) is used to *skip* hashing. If both match the record, the manifest section is fresh.
- `manifest_digest` (blake3 of the file bytes) is the authority when the cheap check says "changed," so a `touch`, a `git checkout` that rewrites mtimes, or a clock skew causes a re-hash and a re-parse of one 800-byte file — not a re-scan.
- A false "changed" is cheap. A false "unchanged" is the dangerous direction, and it has a real cause: filesystems with 1- or 2-second mtime granularity where a write lands in the same second as our record. Mitigation: **any entry whose `generated_at` is within 2 seconds of its recorded `manifest_mtime` is treated as unconditionally stale.** Length comparison catches most of the rest.

**Detection freshness** — `detect_input_digest`:

Detection results are keyed on their actual inputs: a blake3 over the sorted list of `(relative_path, len, mtime)` for every file in the pruned `FileIndex` that any registered detector declared an `Interest` in. If `package.json` changes, or `fly.toml` appears, or `.git` is removed, the digest changes and detection re-runs. If a thousand source files change but no detector input does, it does not.

This is the correct key because it *is* the input set. `git_head_oid` alone would be cheaper but wrong — a dirty working tree changes `package.json` without moving HEAD. HEAD is recorded for display and diagnostics, never as an invalidation input.

`detector_set_version` is an `u32` constant in `vibe-core`, bumped whenever detector logic changes. Upgrading `vibectl` with improved detectors therefore invalidates detection results without invalidating manifest data. Deliberately not derived from the crate version: most releases do not change detector behavior, and invalidating everything on every patch release wastes the user's 2 seconds for nothing.

### 4. `list` may be stale and says so; `show` is never stale

The crisp rule that makes the whole thing safe:

- **`vibe list` answers from cache**, doing only a cheap `stat` per manifest path (50 stats is sub-millisecond). If a stat disagrees with the record, the row is **rendered with a staleness marker** and `--json` carries `"stale": true` — it does *not* silently trigger a re-scan, because a command whose latency depends on invisible state is a bad command. `vibe list --refresh` re-scans.
- **`vibe show <name>` always re-stats, re-hashes, and re-parses the one manifest it is about to display.** It is authoritative by construction. The cache only supplies the name→path lookup.
- **Every write command re-reads the manifest from disk before planning.** No `WritePlan` is ever built from cached content.

So the cache is load-bearing for exactly one thing — making `list` fast — and advisory everywhere else.

### 5. When cache and disk disagree: disk wins, unconditionally

No merge, no prompt, no heuristic. The cache entry is discarded and rebuilt from the manifest. There is no scenario in which a cached value is preferred over an on-disk one.

The one place cached-only data influences behavior is `written_fields` (ADR-0003 §6), which decides whether `vibe sync` may auto-update a field. That is not "cache wins over disk" — disk still supplies the current value; the cache supplies the memory of what *we* last wrote. And it fails closed: no record → treat the on-disk value as user-authored → never overwrite.

**When a manifest has disappeared from disk:** mark the entry `"missing": true` and **keep it**. Do not evict. A project on an unmounted external drive, an unplugged USB disk, or a directory the user temporarily moved is *information*, not garbage, and silently deleting registry entries is a destructive act in a tool whose second hard constraint is "never destructive." `vibe list` shows missing entries dimmed and flagged; `--json` carries the flag.

Entries are therefore never auto-evicted, and the cache grows monotonically with directory churn. At registry scale (hundreds of entries, a few hundred bytes each) that is measured in tens of kilobytes over years. If it ever matters, the answer is that the file is regenerable — delete it.

### 6. Corrupt or unreadable cache: discard silently, never error

A regenerable cache must never be able to block a command. Any failure to open, read, parse, or validate results in an empty in-memory cache and a single `Diagnostic` at `Severity::Note`.

The API refuses to let a caller treat this as an error:

```rust
// vibe-core/src/cache/mod.rs
pub enum CacheLoad {
    Loaded(Cache),
    Rebuilt { cache: Cache, reason: RebuildReason },
}
pub enum RebuildReason { Absent, Unparseable, TruncatedWrite, VersionUnsupported, PathMismatch }

impl Cache { pub fn load(paths: &Paths) -> CacheLoad; }   // note: not Result
```

Returning `CacheLoad` rather than `Result<Cache, _>` means there is no `?` that can propagate a cache problem into a user-visible failure, and the caller is forced to acknowledge the rebuild (to report it) rather than ignore it.

### 7. A cache from a future version: version the filename, don't fight over the file

`cache_version` is independent of the manifest `schema_version` (ADR-0002) and bumps freely — it is our private format.

The cache file is **named by version**: `index.v1.json`, `index.v2.json`. Each build reads and writes only its own file and ignores all others. A newer `vibectl` never sees a stale v1 file; an older one never sees the v2 file.

The alternative — one filename, discard on version mismatch — causes cache thrash: a user with both a stable and a nightly `vibectl` would have each build destroy the other's cache on every invocation, permanently paying full scan cost. Versioned filenames cost a few orphaned kilobytes and eliminate the thrash entirely. Old files are cleaned up opportunistically on write: on a successful save, remove `index.v*.json` files with a version below ours.

**Note the deliberate asymmetry with ADR-0002.** A manifest from the future is *preserved and refused* (it is the user's data; destroying it is unthinkable). A cache from the future is *ignored* (it is ours, and regenerable). Same input, opposite response, because ownership differs.

### 8. Writes are atomic; concurrency is last-writer-wins

Save writes `index.v1.json.tmp-<pid>` in the same directory, flushes and fsyncs, then `std::fs::rename`s over the target — which replaces an existing file on Windows as well as Unix. A truncated temp file is never observable as the cache, and a crash mid-save leaves the previous cache intact.

**No lock file.** Two concurrent `vibe scan` runs mean one loses its update. We are accepting that rather than adding advisory file locking, because the lost data is regenerable by definition, and because a stale lock file left by a killed process is a worse user experience (a tool that hangs or refuses to run) than a cache that occasionally needs re-warming. Orphaned `.tmp-<pid>` files are cleaned on the next successful save.

## Consequences

**Easier:** `vibe list` is a JSON parse plus 50 stats — comfortably under 50ms. The cache can be deleted at any moment, by the user or by us, with zero risk; this makes "have you tried deleting the cache?" a legitimate and safe support answer. Detection and manifest invalidation being separately keyed means a manifest edit does not pay for re-detection and a `npm install` does not pay for a re-parse. Testing is straightforward: the invalidation logic takes witnesses in and a boolean out, with no filesystem needed.

**Harder:** Two versioned witnesses per entry plus provenance is more bookkeeping than "just re-scan," and every new cached field needs a decision about which witness governs it. Bumping `detector_set_version` is a manual step that a contributor will forget, shipping improved detectors that do not take effect until something else invalidates — worth a checklist item in the detector-authoring docs, and worth a test that fails if `DetectorSet::builtin()`'s composition hash changes without the constant moving.

**Trade-off accepted #1:** *`vibe list` can show stale data.* We chose speed plus an honest staleness marker over always-correct-but-slow. A user who edits `.vibe/project.toml` by hand and immediately runs `vibe list` sees the old description with a marker next to it, and must run `vibe list --refresh` or `vibe show` to see the new one. This is the right call for a command that will be run twenty times a day, and it is consistent with ADR-0003's principle that a labelled gap beats a confident wrong answer — but it will surprise someone at least once.

**Trade-off accepted #2:** *Missing entries are never evicted,* so the cache accumulates ghosts from deleted or renamed directories, and `vibe list` gets slowly noisier for a user who churns project locations. We chose that over any automatic eviction rule, because every such rule is a heuristic about whether the user's data is gone forever, and being wrong means silently deleting a registry entry. The v1 CLI surface has no `vibe forget`; adding one is the obvious v2 fix.

**Trade-off accepted #3:** *Concurrent scans can lose an update,* and there is no locking to prevent it. Regenerable data justifies the simplicity; a hung tool waiting on a stale lock does not.
