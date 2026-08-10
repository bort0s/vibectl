# ADR-0005: Core API Amendments for Desktop Consumption

## Status

Accepted (2026-08-10). **Amends [ADR-0001](0001-crate-boundaries.md).**

## Context

ADR-0001 designed `vibe-core` to be reusable from a future Tauri frontend and
closed with a list of assumptions flagged for a specialist to check. That review
happened before any code was written. It confirmed the two load-bearing choices
— a synchronous API and a caller-injected `Reporter` — and found seven places
where the design is CLI-shaped in ways that are free to fix now and breaking to
fix later, plus one outright security defect.

Everything here is recorded now precisely because none of it exists yet. Each
item states the cost of doing it now versus after P1–P3 have shipped.

## Decision

### 1. `WritePlan` is `Serialize` only — never `Deserialize`

**This is a security fix, not an ergonomic one.**

`WritePlan` contains `CreateFile { path, contents }` and `RunCommand { program,
args, cwd }`. If a frontend can round-trip a plan out to the webview and back
into `apply()`, then `apply()` accepts arbitrary file writes and arbitrary
process execution *as data from the renderer*. One XSS in a project description,
or one compromised dependency in a frontend bundle, becomes code execution.
Re-verifying `Precondition`s does not help — a fabricated plan simply omits them.

`WritePlan`, `FileOp`, and `Precondition` therefore derive `Serialize` and **not**
`Deserialize`. Nothing in v1 needs the inverse: `--dry-run --json` only
serializes, and `insta` snapshots only serialize. The derive would exist solely
to enable the one pattern that must not happen.

The GUI flow this forecloses has a safe replacement, deliberately **not** built
now because it is purely additive: `plan_*()` returns an opaque `PlanId` plus a
serializable preview, the `Registry` retains the real plan, and `apply_by_id()`
looks it up. Removing a public trait impl later is a breaking change; declining
to add it costs nothing.

Independently, `apply()` validates at the existing precondition site that every
op path resolves under a configured root or the plan's declared target
directory, and that every `RunCommand.program` is in a fixed `{git, gh}`
allowlist. This turns "we never generate a bad plan" from a convention into an
assertion, which is worth having for the CLI regardless of any frontend.

### 2. Cancellation is a success outcome, not an error

`Reporter::should_cancel()` firing must never produce `Err`. In a CLI, cancel is
Ctrl-C and becomes an exit code; in a GUI it is a normal button and must not
render as an error dialog, and the projects already found must survive.

```rust
pub struct ScanReport  { /* … */ pub outcome: ScanOutcome }
pub struct ApplyReport { /* … */ pub outcome: ApplyOutcome }

pub enum ScanOutcome  { Completed, Cancelled }
pub enum ApplyOutcome { Completed, Cancelled { after_ops: usize } }
```

`ApplyOutcome::Cancelled` carries the op count because ADR-0001 already accepts
that `apply` is not filesystem-atomic. Under a GUI, a partial apply stops being
a crash-only edge case and becomes a routine outcome, so the report must let the
caller say "created 3 of 7 files, re-run to finish". `ApplyReport { applied,
skipped }` alone cannot distinguish user cancellation from precondition skips.

*Now:* free. *Later:* breaking twice — a new public field plus a flipped
`Result` contract for every caller.

### 3. `#[non_exhaustive]` on the report and plan structs, `Default` on the input structs

ADR-0001 §4 said "`CoreError` and every event/report **enum**" and missed the
structs. Without this, item 2 is itself a breaking change.

- `#[non_exhaustive]`: `ScanReport`, `ApplyReport`, `WritePlan`, `Diagnostic`,
  `ErrorPayload`, `ProjectSummary`, `ProjectView`, and individually on
  named-field enum variants such as `Event::ProjectDone`.
- `Default` + `with_*` setters: `Config`, `Query`, `ScanRequest`, `NewRequest`.
  These are constructed *by* the caller, so `#[non_exhaustive]` would be hostile;
  they need the opposite treatment so a future field does not break every
  struct literal. A Tauri app in particular needs a fully custom `Config`, not
  `Config::discover()`.

*Later:* adding `#[non_exhaustive]` is itself a breaking change, so the door
closes permanently.

### 4. `ProjectId` derives from path bytes, never from a display string

ADR-0001 flagged non-UTF-8 paths as an unresolved risk. The display half is
minor — macOS enforces UTF-8, Windows lossiness needs unpaired surrogates. The
**addressing** half is not: if a GUI round-trips a lossy path as an identifier,
`show()` either misses the project or, once two paths collapse to the same
replacement-character string, silently operates on the *wrong* one.

`ProjectId` is therefore a hex-rendered hash of the canonical path's raw bytes
(`as_encoded_bytes()`), and `ProjectRef::Id` is the addressing mode any
programmatic consumer uses. Lossiness is confined to labels, where a garbled
character is cosmetic.

For display, `--json` carries `"path"` as a lossy string plus a sibling
`"path_lossy": true` with `skip_serializing_if`, so it is invisible to `jq` in
the normal case.

*Later:* a `cache_version` bump and every persisted selection invalidated.

### 5. No `Duration`, `Instant`, or bare `OsString` in any `Serialize` type

`Duration` serializes as `{"secs":0,"nanos":123000000}` — bad in a webview and
bad for `jq`. `OsString`'s serde impl is platform-tagged and does not round-trip
across platforms; some formats reject it outright.

- `Event::ProjectDone` / `ScanFinished` carry `elapsed_ms: u64`.
- `FileOp::RunCommand` keeps `OsString` in the in-memory type and projects to
  display strings in the wire shape.

This fixes `vibe new --dry-run --json` today, with or without a frontend.

### 6. `ErrorPayload` and `DiagnosticDetail` are `{ code, params }`

ADR-0001's "core never writes human prose" is right, but as stated it means every
consumer reimplements `vibectl`'s whole message catalog. The wire shape is a
stable `code` plus **named interpolation params**, with prose carried alongside
rather than instead. A frontend then drives an i18n catalog keyed on `code`
instead of hand-writing a match arm per variant.

This also dodges a real bug: `ErrorPayload.chain` on Windows contains
OS-locale-dependent text, because `FormatMessage` returns localized strings. The
structured discriminants (`io::ErrorKind` as a stable string, `status` for
`ToolFailed`, `found`/`supported` for `SchemaTooNew`) must travel next to the
prose so a consumer can branch on them.

### 7. `RwLock` poisoning is recovered from, and the cache guard is scoped tightly

A CLI panic ends the process, so lock poisoning is unobservable. A GUI survives
the panic — Tauri turns it into a `JoinError` — leaving `RwLock<Cache>` poisoned
for the life of the process, so every subsequent `list` and `show` fails until
restart. One panicking detector bricks the app.

Lock acquisition uses `.unwrap_or_else(|e| e.into_inner())` (or `parking_lot`,
decided at implementation time). Separately, `scan` takes the write guard only
to swap in the finished cache, never across the walk — otherwise every `list`
from a UI blocks for the full scan duration and the window freezes.

Note this is a different problem from ADR-0004 §8, whose last-writer-wins
reasoning is about *cross-process* concurrency. A GUI introduces *intra-process*
concurrent scans sharing one lock.

### 8. Extend the `Send + Sync + 'static` assertion beyond `Registry`

`WritePlan`, `ScanReport`, `ApplyReport`, and `Config` all cross a
`spawn_blocking` boundary. They go in the same compile-time assertion.

### 9. Two documentation obligations, recorded so they are not lost

- **`Reporter::event()` is called concurrently from rayon workers, so event
  ordering is not guaranteed.** A consumer must not assume `ProjectFound`
  precedes its matching `ProjectDone` across threads. This belongs in the trait
  docs; it is not fixable in the API without serializing the walk.
- **The only correct Tauri wiring is an explicit
  `tauri::async_runtime::spawn_blocking`.** A plain `#[tauri::command]` runs on
  the *main thread* and freezes the window for the whole scan;
  `#[tauri::command(async)]` moves it to a tokio worker, which is better and
  still wrong. This is a docs problem, not an API problem.

## Rejected

- **Making `vibe-core` async.** `scan` is filesystem-, CPU-, and
  subprocess-bound with no I/O concurrency to multiplex, so `async` buys nothing
  and forces a runtime onto the CLI and every future embedder. Confirmed, not
  merely assumed.
- **`Registry: Clone` / internal `Arc`.** A consumer does
  `app.manage(Arc<Registry>)` and clones the `Arc`. Adding `Clone` later is
  non-breaking, so adding it speculatively is unjustified.
- **Reporter backpressure or event coalescing in core.** 50 projects is ~100
  events in 2 seconds. A pathological root pointed at `~` is a consumer-side
  coalescing problem.

## Consequences

**Easier:** the `--json` contract stops carrying types that serialize badly, so
it is fit for `jq` on day one rather than after a wire break. Cancellation
becomes expressible, which the CLI needs for Ctrl-C anyway. `apply` gains path
and program assertions that make the "never destructive" constraint checkable
rather than merely intended.

**Harder:** more attributes and more ceremony on every public type, and the
`{code, params}` diagnostic shape is more work per variant than a formatted
string would have been.

**Trade-off accepted:** *the safe GUI plan-apply path (`PlanId` +
`apply_by_id`) is designed but not built.* A frontend cannot execute a plan it
received over IPC until that lands. This is the correct order — the additive
feature can arrive whenever a frontend does, whereas the `Deserialize` derive it
replaces could not be removed once shipped.
