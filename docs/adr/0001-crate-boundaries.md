# ADR-0001: Crate Boundaries Between `vibe-core` and `vibectl`

## Status

Accepted (2026-08-10). **Amended by
[ADR-0005](0005-core-api-amendments-for-desktop-consumption.md)**, which
resolves the Tauri assumptions flagged at the end of this document and changes
the `WritePlan`, `FileOp`, `ScanReport`, `ApplyReport`, and `ProjectId` shapes.
In particular **`FileOp::RunCommand` is not `{ program, args }` as shown in §3
below** — ADR-0005 §10 replaces it with a closed set of validated operations,
because a `{git, gh}` program allowlist over free-form argv is not a containment
boundary. The schema error variant in §4 is also superseded: see
[ADR-0002](0002-schema-versioning.md) §3, revised. Read 0005 and the revised 0002
alongside this one before implementing.

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
#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro, clippy::exit)]
```

`unsafe_code = "deny"` is set once in `[workspace.lints.rust]` rather than
`forbid`-ed here. `forbid` cannot be lowered by an inner `#[allow]`, and because
`[lints] workspace = true` is an all-or-nothing inheritance switch, a crate that
ever needed an escape hatch would have to abandon workspace inheritance and copy
the entire table. Some derive macros also expand to `#[allow(unsafe_code)]`,
which is a hard error under `forbid`. `deny` is equally strict by default.

CI runs clippy with `-D warnings`, so a `print!` in core fails the build **there**.
It does not fail a plain `cargo build`: these are clippy lints and rustc ignores
them. They also cover only the `print!` macro family — an explicit
`writeln!(std::io::stdout(), ..)` is not caught by any lint and is a review
obligation. The dependency half of the boundary (no `clap`, no `comfy-table`, no
`anyhow` in core) is asserted by a CI step over `cargo tree -p vibe-core`.

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
- There is deliberately **no `FileOp::Delete`**. `vibe archive` is a single `UpdateFile` op that flips one key in `.vibe/project.toml` (see ADR-0002 for which key).

  **The enforcement claim used to read *"hard constraint 2 is enforced by the absence of the variant — a destructive command is not merely discouraged, it is unrepresentable"*, and that was stronger than what the type system was doing.** *Corrected 2026-08-19.* The absence of `Delete` makes **one form** of destruction unrepresentable: an op that names a path and removes it. It does not make destruction unrepresentable. The write path below passed every file through a **zero-byte state** on every write, with `Delete` nonexistent throughout — so the tool was destroying files by a route the missing variant does not cover.

  The honest form: **`FileOp` has no variant that expresses "delete this path", and that closes deletion-as-an-operation. Everything else about not being destructive is a property of how the ops are carried out, and has to be established there.**

### 3a. DEFECT: every write passed through a zero-byte state, from P0 until 2026-08-19

*Filed 2026-08-19, where the write path lives rather than in the work that found it.*

**What it was.** `apply` wrote `CreateFile` and `UpdateFile` — one shared arm — through `std::fs::write`, which is `File::create` followed by `write_all`. **`File::create` truncates before any byte is written.** So between the truncate and the write the target was **zero bytes**, and that was not a race that might not happen: it was a state the sequence passed through **every single time**.

**How long, and what it reached.** Since the write path was first built. Every command that writes a manifest went through it — `vibe new`, `vibe sync`, `vibe archive`, `vibe render` — and so did the agent-file writes. It was not confined to one caller; it was the primitive.

**What it cost.** A user whose machine lost power, or whose `vibe sync` was killed, in that window had a **zero-byte `.vibe/project.toml`** — their project's manifest, replaced with nothing. No control would have caught it and no message would have said so; the next `vibe list` would simply have found a manifest that no longer parsed. Nobody has reported it, which bounds nothing: the window is short and the population is small.

**Why nobody saw it.** It was invisible in exactly the way this project's failures usually are — the code reads as a write, the tests assert the *result* of the write, and the intermediate state has no observer unless somebody builds one. It surfaced only when ADR-0011's settings editor asked what happens to a file **vibe does not own**, and three rounds had by then gone into whether a killed process could tear a record in a sink where the writer appends and the reader tolerates damage. **The severe hazard was in the primitive the whole time, and the work was aimed at the safer path.**

**The repair.** `write_atomically`: a temporary file **beside the target**, then a rename over it. Beside, not in the system temp directory, because a cross-volume rename is a copy plus a delete. It covers **the primitive**, not the call site that surfaced it — `CreateFile` onto an existing path truncates exactly as `UpdateFile` does, and has its own control saying so.

**Measured, not read.** A reader spinning on the target through 400 replacements sees `Empty` on the old path and only whole contents on the new one, and **the negative half is what licenses the positive one** — a reader too slow to catch anything reports a clean sweep too. Both halves run in the ordinary test job on all three platforms.

**What the repair does not promise.** Durability. There is no `fsync`, so a crash can lose the *new* contents; whether it can also lose the old ones is a property of the filesystem rather than of this code and is not measurable here, so it is not asserted.

**And the second write path had the same shape with a delete in it.** `Cache::save` had its own temp-and-rename plus a fallback that **removed the destination and retried**, under the comment *"Windows will not rename onto an existing file in every case"*. The comment was read rather than measured and the fallback could not help: measured on Windows 10 Pro 19045, a rename-over is refused exactly when another process holds the destination without `FILE_SHARE_DELETE` — and `DeleteFile` is refused in **the same two cases** and permitted in **the one where the rename already worked**. So the fallback was inert where it was aimed and destructive if it had ever fired, since it left the destination missing between the delete and the retry. It now goes through the one primitive.

**The trigger to revisit:** a third write path appearing outside `apply`. There are two today — `apply` and `Cache::save` — and they share one primitive; a third would mean the invariant *"core mutates the filesystem in one place"* has stopped being true, which is the thing §3 exists to say.

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
    // Superseded by ADR-0002 §3 (revised): schema_version is major.minor, and
    // only a *major* mismatch is refused. Now:
    //   SchemaMajorMismatch { path, found: (u16, u16), supported_major: u16 }
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

**What this rule looks like when the second frontend exists, written down before it does.** With one consumer, "core is mute and the CLI speaks" reads as tidiness. With two, it reads as duplication: `vibectl` and the desktop app each write their own sentence for the same `CoreError` or `RemoteBlocked`, and the obvious repair — hoist the string into core so there is one copy — is the wrong one. It is worth stating why now, because the argument is only available before someone is looking at two files that appear to say the same thing.

They do not say the same thing. **What must never be duplicated is the taxonomy — the set of reasons and what each one means — and that is precisely what the enum is, and where it stays.** What is legitimately written twice is *presentation*, and the two presentations are not copies: a CLI's register for "no remote was created" is a paragraph plus paste-ready commands, and a GUI's is a state marker, a disabled action, and a tooltip. Neither is derivable from the other, and a shared string would force one medium into the other's register — usually the GUI into the CLI's, since the CLI is written first.

The test that separates the two cases: **divergence across media is correct; divergence within one medium is drift.** Two frontends rendering the same reason differently is the system working. Two copies of one sentence inside `vibectl` — one in a core `as_str()` and one in the renderer, with the test asserting only the renderer's — is a defect, because nothing keeps them equal and nothing notices when they stop being. That was the actual state of `RemoteBlocked::as_str` and it is what motivated writing this down.

So the direction of repair for a duplicated user-facing string is always **toward the frontend, never toward core**: delete core's copy, let each frontend own its own sentences, and keep in core only the variant that names the reason. A frontend that needs a string for a reason it does not recognise should say it does not recognise it (ADR-0002 §5's treatment of unknown manifest keys), not render a machine key as if it were prose.

**The mechanism that makes a missing sentence break the build, and its limit.** `#[non_exhaustive]` does not constrain the *owning* crate, so core can match its own enums exhaustively: a function mapping variant → stable key, written as an exhaustive `match`, fails to compile the moment a variant is added. A test in each frontend then asserts that every key core exports has a sentence, and goes red. The chain is: new variant → **core** fails to compile → author adds the key → **frontend** test reds → author writes the sentence. That is ADR-0005 §10 rule 4a's discipline — make the omission fail somewhere someone is already looking — in a place where a closed enum is unavailable because the enum must stay open for semver.

Its limit is the enumeration problem wearing a new face, and it is stated rather than assumed closed: **the pattern closes the gap only for the enums it is applied to.** The next enum that crosses to a frontend reintroduces the silent fallback and nothing notices.

**The evidence for that sentence was a live defect, the defect was repaired, and the paragraph went on citing it for two days.** *Re-established 2026-08-13, at `6273a64`, and the previous wording is not merely corrected but re-argued.* It read: *"That is not hypothetical — `vibectl` today has five matches on core enums and eight wildcard arms, and they already disagree about the right degradation: `agents.rs` renders an unrecognised agent state as `"unrecognised"`, while `output.rs` coerces an unrecognised `Severity` to `"warning"`."* The `Severity` half was fixed on 2026-08-11 — before ADR-0009 §4 was written, which records the fix — and this paragraph kept asserting it in the present tense.

**Measured rather than recalled, on that revision.** Every wildcard arm over a core enum in `vibectl`'s sources, classified by hand:

| Site | Enum | Degrades as |
| --- | --- | --- |
| `agents.rs` staleness prose | `Staleness` | *"reported in a form this build does not understand"* |
| `agents.rs` staleness JSON | `Staleness` | `"unrecognised"` |
| `output.rs` apply outcome | `ApplyOutcome` | *"an outcome this build cannot describe"* |
| `output.rs` severity label | `Severity` | *"unrecognised severity `x`"* — the repaired one |
| `output.rs` refusal hint | `RenderState` | *"a reason this build does not understand"*, and declines to promise `--force` |
| `output.rs` hint dispatch | `CoreError` | no hint, so the error's own description stands alone |
| `prompts.rs` exposure label | `IgnoreState` | *"unrecognised state"* |
| `prompts.rs` completeness | `PromptRoot` | *"does not recognise that location"* |

**All of them degrade honestly. None borrows a recognised value.** So the specific claim the old sentence made is false today, and saying so is the whole of the repair to *that* clause.

**No count is given, deliberately, and this replaces one.** *"Five matches and eight wildcard arms"* was a hand-maintained integer of exactly the kind ADR-0008 §9 retired: the person who forgets to update it is the person who would have had to notice it was wrong. The table above is a dated measurement rather than a running total, and it goes stale visibly — a reader can re-derive it — rather than by quietly disagreeing with a number.

**What the table buys and what it does not, in the same three results already measured for `RemoteBlocked::ALL` and for ADR-0002 §7's named-instance list.** This is the identical hole a third time, recorded so nobody reads the table as a closure:

1. **A new wildcard arm added without a row here is caught by nothing.** Markdown has no compiler — and this is *weaker* than `ALL`, which at least fails to build twice before anyone can ignore it. A wildcard arm compiles silently; being uncaught is the entire property that makes it a wildcard.
2. **A table missing a row reads exactly like a complete one.** There is no independent enumeration, so any check driven by this table is circular for the same reason it is for `ALL`: an arm absent from the table is never visited, whatever is asserted about the ones present.
3. **Removing a row is uncaught too.** `ALL` has an array length the type checker enforces; a markdown table has no arity.

So the gain over the integer is real and small, and it is only this: **a wrong table is re-derivable and a wrong integer is not.** A reader who doubts the rows can grep the wildcards and compare; a reader who doubts *"eight"* has nothing to compare it against but the same count they would have to redo anyway. That is a difference in how the staleness is *found*, not in whether it occurs. Completeness here depends on whoever adds the next wildcard remembering, which is the standing cost of every prose index in this repository and is not worth machinery to pretend otherwise.

**What survives, and it is the same claim on better evidence.** The limit holds: eight arms, and only `RemoteBlocked` and `UnknownCause` have the key-plus-test pattern behind them. What changed is the *kind* of evidence, and it changed twice —

- **Weaker in the direction that matters.** *"These wildcards disagree, and one of them lies"* is evidence that the gap **produces defects**. *"One did, and it was fixed"* is much easier to read as solved, and an argument that reads as solved stops motivating the trigger.
- **Stronger, from the session that repaired it.** ADR-0010 phase 3 crossed **two new enums** to a frontend — `IgnoreState` and `PromptRoot` — and added three wildcard arms. They degrade honestly, and **nothing mechanical required them to**: they are honest because the author had read this paragraph. That is better evidence for the limit than the `Severity` defect ever was, because it is present-tense and it is about the *mechanism* rather than about one person's mistake. The gap did precisely what this paragraph predicts, in the ordinary course of adding a feature.

**And the pattern is deliberately not extended to `IgnoreState`, so nobody re-derives it.** An `ALL_KEYS` there would buy nothing: all three variants are constructible by a frontend, so the hand-list in `prompts/tests.rs` is exactly as strong, and neither can reach the wildcard — no constructible input does. `UnknownCause` needs `ALL_KEYS` for a different reason, that two of its variants carry data and cannot sit in a `const` array.

**Decision 4's repair direction does not rest on any of this**, which is worth stating because the two sit four paragraphs apart and the stale line looked load-bearing. *Toward the frontend, never toward core* rests on the taxonomy-versus-presentation argument above — that a CLI's register and a GUI's are not derivable from each other — and on `RemoteBlocked::as_str`, a duplicated string **inside one medium** with the test asserting the wrong copy. Neither is touched by the `Severity` repair, and the second remains true as history whatever the code does now. The stale sentence was evidence for the *mechanism's limit*, not for the direction of repair.

**Measured, not assumed: there is no crate-wide compile-time closure available on stable.** The exact mechanism exists — rustc's `non_exhaustive_omitted_patterns` lint, which fires when a wildcard covers variants that exist in the linked crate but were not listed, leaving the wildcard legal only for genuinely future ones. One crate-level `#![deny(...)]` in each frontend would close this for every enum at once, forever. On stable 1.97.1 it is an **unknown lint**, and because that is a warning it would *fail* the clippy job under `-D warnings`. (It is understood to be nightly-gated behind `non_exhaustive_omitted_patterns_lint`; that half is recalled, not measured — no nightly toolchain was available here to confirm it.)

So the honest ordering, until that lint stabilises: apply the key-plus-test pattern to the enums that cross today, state the gap, and treat **"the lint stabilises"** as the trigger that replaces the pattern with one attribute. A macro in core defining enum, key and key-list together is the nearest mechanical alternative and is *not* taken at two enums — it moves the thing to remember from "write a test" to "use the macro", which is a real improvement but not a compile-time guarantee, and it is the kind of machinery this project declines to build before its third caller.

`CoreError` and every event/report enum are `#[non_exhaustive]`, since they are public API under semver.

**Confirmed 2026-08-11, and given the trigger it was missing.** That sentence rests on the Context's "library, published as `vibe-core`" — an intent recorded once and never revisited, which is the shape this project has now corrected twice (a condition living in someone's memory rather than in an observable event). So:

- **Status: intent, not yet real.** Nothing is on crates.io. The only named consumers are `vibectl` and the planned Tauri frontend, both in-workspace and both consumed by path. No external consumer exists.
- **The trigger, whichever fires first:** a consumer outside this workspace; a `v1.0.0` tag; or the decision that the frontend consume `vibe-core` from crates.io rather than by path. Each is an event that produces a diff or a tag, which is the standard ADR-0008 §9 sets for a revisit trigger — as opposed to "when someone remembers to ask".
- **The asymmetry is why this needs no further argument.** Per ADR-0005 §3, *adding* `#[non_exhaustive]` after first publish is itself a breaking change, so the door closes permanently at that moment; keeping it never closes anything. Worst case the attribute is useless. It buys a permanent option for a cost that the key-plus-test pattern below drives to near zero, so it stays.

This is deliberately **not** treated like the assumptions dismantled elsewhere in this session's ADRs. Those produced green checks that looked like proofs; this produces an attribute that is, at worst, inert. Low stakes, recorded as such.

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
