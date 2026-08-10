# ADR-0001: Crate Boundaries Between `vibe-core` and `vibectl`

## Status

Accepted (2026-08-10)

## Context

The workspace is fixed at two crates: `vibe-core` (library, published as `vibe-core`) and the CLI (`vibectl`, binary `vibe`). `vibe-core` must contain manifest types, parsing, detection, and the render engine, with **no stdout I/O and no clap**, because a future Tauri desktop app will consume it as a library.

"No stdout I/O" is easy to state and easy to violate accidentally. Two features make the seam non-obvious:

1. **`vibe scan ~/projects` needs progress reporting.** Indexing 50 repos in under 2 seconds still means 2 seconds of silence unless something streams. Core owns the loop; the CLI owns the terminal.
2. **`--dry-run` on every write command needs the *intent* of a write described before it happens.** If core writes files directly, then dry-run must be implemented by threading a boolean through every write site, and the first site that forgets it is a data-loss bug in a tool whose second hard constraint is "never destructive."

There is also an error-reporting seam: `anyhow` and `thiserror` are both in the dependency set, and it must be unambiguous which crate uses which.

Note that "no stdout I/O" does **not** mean "no I/O." Core reads the filesystem and spawns `git`/`gh` — that is its job. The prohibition is on the *terminal*, and on any decision about how a human should be addressed.

## Decision

### 1. The seam, stated as an invariant

- **`vibe-core` never touches stdout, stderr, the terminal, the process exit code, or the environment's presentation state (colors, TTY width, locale).** It never formats a sentence intended for a human to read as prose.
- **`vibectl` never touches the filesystem inside a project directory, never spawns `git` or `gh`, and never parses TOML.** Every byte it displays came out of a `vibe-core` return value.

Enforced mechanically, not by discipline, in `vibe-core/src/lib.rs`:

```rust
#![forbid(unsafe_code)]
#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro, clippy::exit)]
```

CI runs clippy with `-D warnings`. A `print!` in core fails the build.

### 2. Progress and diagnostics: a `Reporter` sink injected by the caller

Core pushes events into a caller-supplied sink. It does not return an iterator of events, and it does not spawn threads to do it.

```rust
// vibe-core/src/report.rs
pub trait Reporter: Send + Sync {
    fn event(&self, ev: Event);
    /// Cooperative cancellation, polled at every project boundary.
    fn should_cancel(&self) -> bool { false }
}

#[non_exhaustive]
pub enum Event {
    ScanStarted   { roots: Vec<PathBuf> },
    RootEntered   { root: PathBuf },
    ProjectFound  { path: PathBuf, name: Option<String> },
    ProjectDone   { path: PathBuf, elapsed: Duration },
    ScanFinished  { found: usize, elapsed: Duration },
    Diagnostic(Diagnostic),
}

pub struct Diagnostic {
    pub severity: Severity,       // Note | Warn
    pub code: &'static str,       // stable, e.g. "VIBE_W_DETECT_CONFLICT"
    pub subject: Option<PathBuf>,
    pub detail: DiagnosticDetail, // structured enum, NOT a preformatted string
}

pub struct NullReporter;         // default, zero-cost
pub struct CollectingReporter;   // Mutex<Vec<Event>>, for tests and --json
```

`Diagnostic::detail` is a structured enum, not a `String`. Core states *what happened*; `vibectl` decides the wording, the color, and whether it is worth showing at the current verbosity.

Diagnostics are emitted through the `Reporter` **and** accumulated into the returned report struct (`ScanReport.diagnostics`). The duplication is deliberate: the stream drives live UX, the aggregate drives `--json` and exit-code decisions, and a caller that passes `NullReporter` must still get the full picture.

Cancellation lives on the same trait rather than a separate `CancelToken` parameter. This conflates two concerns in one trait, which is a real wart, but it keeps every long-running core method to a single extra argument.

### 3. Writes: plan/apply, with no third option

Core exposes **no method that mutates the filesystem except `apply`.** Every write command is two calls.

```rust
// vibe-core/src/plan.rs
#[must_use]
pub struct WritePlan {
    pub intent: PlanIntent,     // New | Sync | Render | Archive
    pub ops: Vec<FileOp>,
}

#[non_exhaustive]
pub enum FileOp {
    CreateDir  { path: PathBuf },
    CreateFile { path: PathBuf, contents: String },
    UpdateFile { path: PathBuf, before: String, after: String, reason: EditReason },
    RunCommand { program: OsString, args: Vec<OsString>, cwd: PathBuf, why: CommandReason },
}

/// Recorded at plan time, re-verified at apply time.
pub struct Precondition { pub path: PathBuf, pub digest: Option<[u8; 32]> }

pub struct ApplyReport { pub applied: Vec<AppliedOp>, pub skipped: Vec<SkippedOp> }
```

- `--dry-run` is `plan_*()` then render the plan. It is not a flag core knows about.
- `UpdateFile` carries both `before` and `after` so `vibectl` can render a real diff without re-reading the file.
- Each op carries `Precondition`s captured at plan time. `apply` re-hashes and **aborts the whole plan** if any target changed since planning. Apply is all-or-nothing at the *decision* level; it is not transactional at the filesystem level (see Consequences).
- `FileOp::RunCommand` exists so `vibe new`'s `git init` / `gh repo create` show up in a dry run instead of being invisible side effects.
- There is deliberately **no `FileOp::Delete`**. Hard constraint 2 is enforced by the absence of the variant — a destructive command is not merely discouraged, it is unrepresentable. `vibe archive` is a single `UpdateFile` op that flips one key in `.vibe/project.toml` (see ADR-0002 for which key).

### 4. Errors: `thiserror` in core, `anyhow` in the CLI, no exceptions

```rust
// vibe-core/src/error.rs
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    #[error("manifest not found at {path}")]
    ManifestNotFound { path: PathBuf },
    #[error("manifest at {path} is not valid TOML")]
    ManifestSyntax { path: PathBuf, span: Option<Range<usize>>, #[source] source: toml_edit::TomlError },
    #[error("manifest at {path} declares schema_version {found}, this build supports {supported}")]
    SchemaTooNew { path: PathBuf, found: u32, supported: u32 },
    #[error("`{program}` is not on PATH")]
    ToolMissing { program: &'static str },
    #[error("`{program}` exited with status {status}")]
    ToolFailed { program: &'static str, status: i32, stderr_excerpt: String },
    #[error("{path} changed on disk between plan and apply")]
    PlanStale { path: PathBuf },
    #[error("io error at {path}")]
    Io { path: PathBuf, #[source] source: std::io::Error },
}

impl CoreError {
    /// Stable across releases. Drives `--json` and exit codes.
    pub fn code(&self) -> &'static str { /* "VIBE_E_SCHEMA_TOO_NEW", ... */ }
    /// Serializable projection; the error enum itself is NOT Serialize.
    pub fn to_wire(&self) -> ErrorPayload { /* code, message, path, chain: Vec<String> */ }
}
```

Core errors carry *data* (paths, spans, key names, exit statuses), never a human-facing remediation sentence. `vibectl` owns "did you mean", "run `vibe scan` first", and color. `anyhow` appears only in `vibectl`; `vibe-core` has zero `anyhow` in its public API, so a `?` in core cannot accidentally erase structure.

`CoreError` and every event/report enum are `#[non_exhaustive]`, since they are public API under semver.

### 5. Top-level API: a `Registry` handle, not free functions

Free functions would force every call site to re-thread the roots list, the cache path, the `git` binary location, and the clock. A handle carries them and makes them injectable for tests.

```rust
// vibe-core/src/registry.rs
pub struct Registry { /* Config, Paths, DetectorSet, RwLock<Cache>, Arc<dyn ProcessRunner> */ }

impl Registry {
    pub fn open(config: Config) -> Result<Registry, CoreError>;

    // Reads — cheap, cache-backed, no Reporter needed.
    pub fn list(&self, q: &Query) -> Result<Vec<ProjectSummary>, CoreError>;
    pub fn show(&self, sel: &ProjectRef) -> Result<ProjectView, CoreError>;

    // The expensive read.
    pub fn scan(&self, req: &ScanRequest, rep: &dyn Reporter) -> Result<ScanReport, CoreError>;

    // Writes — always planned first.
    pub fn plan_new(&self, req: &NewRequest) -> Result<WritePlan, CoreError>;
    pub fn plan_sync(&self, sel: Option<&ProjectRef>, rep: &dyn Reporter) -> Result<WritePlan, CoreError>;
    pub fn plan_render(&self, sel: &ProjectRef, target: RenderTarget) -> Result<WritePlan, CoreError>;
    pub fn plan_archive(&self, sel: &ProjectRef) -> Result<WritePlan, CoreError>;

    pub fn apply(&self, plan: &WritePlan, rep: &dyn Reporter) -> Result<ApplyReport, CoreError>;
}
```

All methods take `&self`; the cache sits behind an `RwLock` so `Registry` is `Sync`. `Config` is a plain struct with a builder-ish `Config::discover()` that uses `directories`; a caller may override every path, which is how tests get a temp `Config` with no global state.

### 6. `--json` DTOs live in core

`vibe-core/src/json.rs` defines the `Serialize`/`Deserialize` shapes for every read command's `--json` output (`ProjectSummary`, `ProjectView`, `ScanReport`, `ErrorPayload`, `WritePlan`). `vibectl` calls `serde_json::to_writer` on them and adds nothing. This puts a data contract in core, which is presentation-adjacent and slightly uncomfortable — but the Tauri app needs exactly these shapes, and having two definitions guarantees they drift.

### 7. Module tree

```
crates/vibe-core/src/
  lib.rs              # facade re-exports; the lint denies above
  config.rs           # Config, Paths (directories-derived), Clock
  registry.rs         # Registry handle; all top-level operations
  error.rs            # CoreError, ErrorPayload, code()
  report.rs           # Reporter, Event, Diagnostic, Severity, NullReporter, CollectingReporter
  plan.rs             # WritePlan, FileOp, Precondition, ApplyReport
  json.rs             # stable serde DTOs for --json / IPC
  exec.rs             # ProcessRunner trait, SystemRunner (git/gh), OsString arg handling, timeouts
  walk.rs             # `ignore`-based walker, prune set, depth cap, FileIndex
  model/
    mod.rs
    manifest.rs       # Manifest + section structs (read projection only)
    status.rs         # Status enum incl. Status::Other(String)
    detected.rs       # Detected<T>, Confidence, Evidence  (ADR-0003)
    ids.rs            # ProjectId, ProjectRef, slug rules
  manifest/
    mod.rs
    document.rs       # ManifestDocument: the ONLY type that can serialize TOML  (ADR-0002)
    parse.rs          # DocumentMut -> Manifest, tolerant
    version.rs        # schema_version read/migrate  (ADR-0002)
  detect/
    mod.rs            # Detector trait, DetectorSet, DetectCtx
    merge.rs          # FieldPath, MergeKind, conflict resolution  (ADR-0003)
    evidence.rs       # Evidence constructors, Provenance
    stack/            # node.rs cargo.rs python.rs go.rs php.rs
    vcs/git.rs
    deploy/           # vercel.rs netlify.rs fly.rs
  cache/
    mod.rs            # Cache handle, CacheLoad  (ADR-0004)
    schema.rs         # CacheFile, CacheEntry, witnesses
    store.rs          # atomic tmp+rename load/save
  render/
    mod.rs            # RenderTarget, render_* -> WritePlan
    engine.rs         # minijinja Environment; no filesystem loader by default
    templates/        # builtin .j2, include_str!

crates/vibectl/src/
  main.rs             # parse -> dispatch -> map CoreError::code() to ExitCode
  cli.rs              # clap derive; --json / --dry-run defined once as flattened arg groups
  cmd/                # new.rs scan.rs list.rs show.rs sync.rs render.rs archive.rs
  reporter.rs         # TermReporter: Event -> stderr progress; Ctrl-C -> should_cancel
  output.rs           # Emitter { Human(comfy-table), Json(serde_json) }
  ui/                 # table.rs diff.rs theme.rs (color/TTY detection lives here, only here)
  exit.rs             # code() -> ExitCode mapping
```

Progress goes to **stderr**, data to **stdout**, always — so `vibe scan --json > out.json` works with a live progress bar.

## Consequences

**Easier:** `--dry-run` cannot be forgotten, because there is no code path that writes without a `WritePlan` first. `--json` is a serialization of the same values the human path renders, so the two cannot disagree. Core is testable headlessly: `CollectingReporter` plus a fake `ProcessRunner` makes `insta` snapshots of `WritePlan` the primary test artifact — snapshotting a plan is a far better regression test than snapshotting terminal output.

**Harder:** Every write feature costs two methods instead of one. `WritePlan` holds full rendered file contents in memory (fine at manifest/template scale, single-digit KB; it would not be fine if we ever render binaries). Adding a `CoreError` variant or `Event` variant is a semver event.

**Trade-off accepted:** *`apply` is not atomic across multiple files.* Preconditions are checked for all ops before any op runs, so we fail fast on a stale plan, but a crash midway through a multi-file `vibe new` leaves a partial directory. We are accepting this rather than building a journal/rollback layer, because (a) there is no `Delete` op, so a partial apply is always additive and never loses user data, and (b) re-running the command re-plans against the new reality and completes. The cost is that a partial `vibe new` can leave a project directory that looks scaffolded but isn't.

**Second trade-off accepted:** *core is entirely synchronous and blocking.* No `async`, no `tokio`. This is right for a CLI and it keeps the API honest, but it pushes the threading problem onto the Tauri consumer (below).

---

## Note for the Tauri reviewer: is this API actually consumable from a desktop app?

Assumptions I made, flagged for the specialist reviewing this dimension:

- **Sync, not async.** Every `Registry` method blocks. A Tauri command must wrap calls in `tauri::async_runtime::spawn_blocking`. I believe this is correct — `scan` is filesystem- and process-bound, not I/O-concurrency-bound, so making core `async` would buy nothing and would force a runtime dependency on every consumer. **Assumption: the Tauri app is willing to own `spawn_blocking`.**
- **`Send + Sync + 'static`.** `Registry` holds `RwLock<Cache>` and `Arc<dyn ProcessRunner + Send + Sync>`; no `Rc`, no `RefCell`, no raw pointers, no thread-local state. It is intended to live in `tauri::State<Registry>` and be called from many command handlers concurrently. **Unverified assumption: no field I have not yet written breaks auto-`Send`/`Sync`.** This should be pinned by a `const _: fn() = || { fn assert<T: Send + Sync + 'static>() {} assert::<Registry>(); };` compile-time assertion in `registry.rs`.
- **Progress across the IPC boundary.** `Reporter` is `Send + Sync`, so a Tauri impl that calls `window.emit("vibe://scan", ev)` works. This requires `Event` and `Diagnostic` to be `Serialize`, which they are (they live in the `json.rs` contract set). `should_cancel()` reads an `AtomicBool` the frontend flips via a second command.
- **Errors across IPC.** `CoreError` is deliberately **not** `Serialize` — `#[source]` chains containing `std::io::Error` and `toml_edit::TomlError` are not serializable. `to_wire() -> ErrorPayload` is the boundary type. **Assumption: the Tauri app is fine converting at the boundary rather than getting `Serialize` for free.**
- **Returned types.** `ProjectSummary`, `ProjectView`, `ScanReport`, `WritePlan`, `ApplyReport` are all `Serialize + Deserialize`, no lifetimes, no borrowed data. They contain `PathBuf`, which serializes as a string and is **lossy for non-UTF-8 paths on Linux** — an unresolved risk for both the JSON output and the IPC boundary. I have not decided how to handle non-UTF-8 paths; flagging it rather than pretending it's solved.
- **Plan/apply suits a GUI better than it suits the CLI.** A desktop app naturally wants "show me what will change, let me confirm." `plan_*` → render diff → `apply` is exactly that flow, for free.
