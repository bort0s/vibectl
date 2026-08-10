# ADR-0003: Detection Model — Detectors, Composition, and Compiler-Enforced Honesty

## Status

Accepted (2026-08-10). **Amended (2026-08-10)** after implementation and two
adversarial reviews. Changes, each recorded at the section it affects:

- §2 — `Specificity` moved from the `Detector` trait to the individual
  `Finding`.
- §7 — `GitRepo` no longer produces `dirty` or `branch`; both are removed from
  the inventory rather than left as unimplemented spec lines.
- §8 — parallelism is `std::thread::scope` with a shared work cursor, not
  rayon and not static chunks. `git` is invoked **twice** per repository, not
  once. The measured practical ceiling is **~150 projects**.

## Context

`vibe scan ~/projects` is the differentiating command. It walks directories that already exist and infers stack, git state, and deploy targets. It must handle 50 repos in under 2 seconds, and it must obey hard constraint 5: **detection is honest — when the stack cannot be inferred, write an empty field and flag it; never guess, never invent a plausible-looking value.**

The failure mode is specific and it is *seductive*. Every detector author, at 11pm, will hit a directory with a `Dockerfile` that mentions `python:3.11` and no `pyproject.toml`, and will think: "well, it's obviously Python." Then a user's registry claims a runtime that isn't there, they trust it, and the tool's one differentiating feature is a liar. The same pressure produces `unwrap_or_default()`, `"unknown"` sentinel strings, and `Option::unwrap_or("node")`.

A convention ("please don't guess") does not survive contact with the tenth detector. This ADR is mostly about making the honest path the only one the compiler will accept.

There is also a composition problem: several detectors fire on one directory (a repo can have `package.json`, `Cargo.toml`, `.git`, and `vercel.json`), they disagree, and detection must not silently overwrite something the user typed by hand.

## Decision

### 1. Honesty is enforced by making *evidence* a required constructor argument

The usual design is `Option<T>` plus a rule about not guessing. That fails because `Option` has `unwrap_or_default`, `Default`, and a hundred ergonomic escape hatches.

```rust
// vibe-core/src/model/detected.rs
#[must_use]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Detected<T> {
    Known { value: T, confidence: Confidence, evidence: Evidence },
    Unknown { reason: UnknownReason },
}
```

The variant is the *smaller* half of the mechanism. The real enforcement:

- **`Detected<T>` implements no `Default`, no `Deref`, no `From<Option<T>>`, no `unwrap_or*`.** There is no ergonomic path from "I have nothing" to "I have a value." Consuming a `Detected` requires a `match`.
- **`Detected::known()` cannot be called without an `Evidence`,** and `Evidence` has no public constructor that does not name a source:

```rust
// vibe-core/src/detect/evidence.rs
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence { source: EvidenceSource, locator: String, excerpt: String }

impl Evidence {
    /// `locator` is a JSON pointer / TOML path / line number into `file`.
    pub fn from_file(file: &Path, locator: impl Into<String>, excerpt: impl Into<String>) -> Evidence;
    /// `cmd` is the exact argv; `excerpt` is the captured output that justified the value.
    pub fn from_command(cmd: &[OsString], excerpt: impl Into<String>) -> Evidence;
}
```

There is no `Evidence::none()`, no `Evidence::default()`, no `Evidence::inferred()`. **You cannot construct a detected value without pointing at the bytes that justify it.** That is the whole trick: honesty stops being a rule about intent and becomes a rule about what typechecks. The 11pm Dockerfile case still compiles — you can cite the Dockerfile line — but you must cite it, and the citation is what forces you to also pick a `Confidence`, which is what stops it reaching disk (§3).

Supporting lints in `detect/`: `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::or_fun_call)]`.

`UnknownReason` is a closed enum, because "why don't we know" is displayable information:

```rust
pub enum UnknownReason {
    NoEvidence,                                   // nothing on disk spoke to this field
    Conflict { candidates: Vec<ConflictCandidate> },
    LowConfidenceOnly { candidates: Vec<ConflictCandidate> },
    Unreadable { path: PathBuf },                 // permissions, malformed JSON
    Timeout { detector: DetectorId },
    NotAttempted { detector: DetectorId, why: SkipReason }, // e.g. `git` not on PATH
}
```

`Unknown` is never rendered as `"unknown"` or `"n/a"` in the manifest. Per constraint 5, the field is written **empty** (`runtime = ""`, or the key omitted for optional tables) and the reason is surfaced in the scan report and in `vibe show`.

### 2. The `Detector` trait

```rust
// vibe-core/src/detect/mod.rs
pub trait Detector: Send + Sync {
    fn id(&self) -> DetectorId;                    // stable str, appears in every Evidence
    fn specificity(&self) -> Specificity;          // tie-break rank, see §5
    /// What this detector needs to exist before it is worth running.
    fn interest(&self) -> &'static [Interest];
    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError>;
}

pub enum Interest { FileName(&'static str), Extension(&'static str), DirName(&'static str) }

pub struct Finding {
    pub field: FieldPath,
    pub value: FieldValue,
    pub confidence: Confidence,
    pub evidence: Evidence,
    pub detector: DetectorId,
}
```

`interest()` is the performance contract. **No detector walks the filesystem itself.** `walk.rs` walks each project directory exactly once (via `ignore`, pruning `node_modules`, `target`, `.git`, `dist`, `vendor`, `build`, `.next`, `venv`, `__pycache__`), producing a `FileIndex`. The `DetectorSet` then invokes only those detectors whose `interest()` intersects the index. On a Go repo, the five Node/Python/PHP detectors never run and never touch the disk.

`DetectCtx` is the only I/O surface a detector gets:

```rust
pub struct DetectCtx<'a> {
    pub root: &'a Path,
    pub files: &'a FileIndex,
    pub deadline: Instant,
    exec: &'a dyn ProcessRunner,      // git/gh, with a hard per-call timeout
    reads: &'a ReadCache,             // memoized, size-capped
}

impl DetectCtx<'_> {
    pub fn read_text(&self, rel: &Path) -> Result<Arc<str>, DetectError>;  // cap: 1 MiB
    pub fn git(&self, args: &[&str]) -> Result<Output, DetectError>;
}
```

`read_text` is memoized per directory, so `package.json` is read once even though three detectors want it. It is size-capped so a pathological 400 MB `package.json` cannot blow the budget.

**`.env` is on a hard never-read list**, alongside `*.pem`, `*.key`, `id_rsa*`, `.npmrc`, `.netrc`, and `credentials`. `deploy.env_required` is derived from `.env.example` / `.env.sample` **key names only**. The registry indexes secrets' *names*, never their values, and cannot leak a secret into a manifest that gets committed. `ReadCache` refuses these paths at the API level, so no detector can opt in.

### 3. `Confidence` is three levels, and each level has a distinct behavior

```rust
pub enum Confidence { Certain, Likely, Weak }
```

Three ordinal levels, not a float. A float invites `0.73` — fake precision, and arithmetic on it (averaging two guesses!) means nothing. Three levels are defensible because each one maps to a *different action*:

| Level | Meaning | What happens to it |
|---|---|---|
| `Certain` | The authoritative file for this fact states it directly | Written to the manifest |
| `Likely` | Strong circumstantial evidence, authoritative source absent | Written to the manifest, plan op tagged `EditReason::Inferred` so `--dry-run` shows it as inferred |
| `Weak` | Plausible, not corroborated | **Never written.** Surfaced in the scan report as a suggestion the user can confirm |

Examples: `"react": "^19.0.0"` in `package.json` dependencies → `Certain` (`react@19`). `vite.config.ts` present with no `vite` dependency → `Likely` (vite). `FROM python:3.11` in a Dockerfile with no `pyproject.toml`, no `requirements.txt`, no `.py` in the index → `Weak`, and it does not reach disk.

This is the rule that makes `Confidence` load-bearing instead of decorative, and it is the second half of the honesty enforcement: **a `Weak` finding cannot become a manifest value, no matter what a detector author intended.** The gate lives in `merge.rs`, once, not in each detector.

A detector signals low confidence purely by choosing the level; there is no separate "warning" channel. If it wants to explain itself, the explanation belongs in `Evidence::excerpt`, which the user can see in `vibe show --json`.

### 4. Composition: independent detectors, one merge pass

Detectors are mutually blind. None reads another's output, none runs conditionally on another's result, and their execution order does not affect the outcome. `DetectorSet::run` collects every `Finding` into a flat `FindingSet`, and a single `merge.rs` pass resolves it into `Detected<T>` fields.

Order-independence is what makes the system testable: a detector's unit test is a fixture directory in, `Vec<Finding>` out — no orchestration, no mocking of peers.

`FieldPath` is an **enum, not a string**:

```rust
pub enum FieldPath {
    ProjectDescription, StackRuntime, StackFramework, StackService,
    RepoRemote, RepoVisibility, DeployUrl, DeployEnvRequired,
    GitLastCommit, GitBranch, GitDirty,
}
```

Adding a manifest field forces every `match` in the merge pass to be reconsidered. A string key would let a new field silently fall through to a default branch — which is precisely how "never guess" erodes.

### 5. Conflict resolution: a deterministic ladder that ends in `Unknown`

Each `FieldPath` declares a merge kind:

```rust
pub enum MergeKind { Scalar, Set }
```

- **`Set` fields union.** `stack.frameworks`, `stack.services`, `deploy.env_required` collect every `Certain`/`Likely` finding, dedupe, and sort deterministically. A repo with React *and* FastAPI legitimately has both; there is no conflict to resolve. Sorting is required for stable `insta` snapshots and to keep `vibe sync` from producing a diff on every run.
- **`Scalar` fields compete.** `stack.runtime`, `repo.remote`, `repo.visibility`, `deploy.url` have exactly one true value, so disagreement is real. The ladder, applied in order:

1. **Drop all `Weak` findings.** If only `Weak` remains, the field is `Unknown { LowConfidenceOnly { candidates } }` and the candidates are reported as suggestions.
2. **Higher `Confidence` wins.** `Certain` beats `Likely`, always.
3. **Tie on confidence → higher `Specificity` wins.** `Specificity::Lockfile > Manifest > Config > Heuristic`. `pnpm-lock.yaml`'s resolved `22.11.0` beats `package.json`'s `"engines": { "node": ">=20" }`, because a lockfile records what is actually installed and a range records a wish.
4. **Still tied and the values differ → we do not pick.** The field becomes `Detected::Unknown { reason: Conflict { candidates } }`, is written empty, and the conflict is reported with all candidates and their evidence so the user can resolve it by hand.

Step 4 is the honesty rule applied to ambiguity: **a coin flip is a guess.** A tool that reports "I found both `node@22` and `python@3.12` and can't tell which is the runtime — here's where I saw each" is more useful than one that confidently picks the alphabetically first.

Resolution is deterministic and contains no timestamps, no filesystem iteration order, and no HashMap iteration — two scans of an unchanged directory produce byte-identical results, or it is a bug.

### 6. Detection never overwrites a human

Detection results must not fight the user's hand edits, and the manifest schema (fixed by the product owner) has no room for per-field provenance. So provenance lives in the cache (ADR-0004): for each field we last wrote, we record the exact value we wrote.

`vibe sync` updates a field only when:

- the on-disk value is empty or absent, **or**
- the on-disk value is byte-identical to what we recorded writing last time (so it is ours, and stale).

If the on-disk value differs from our record, **the user (or an agent, or a merge) edited it.** Sync does not touch it; it reports the divergence and offers the detected value as a suggestion in the scan report.

Critically, this **fails closed when the cache is missing**: no record → treat as user-authored → never overwrite. A user who deletes their cache loses convenience, never content. That property is the reason provenance is allowed to live in a regenerable cache at all.

### Amendment: fail-closed extends to a failed *detection*, not only a missing cache

The rule above answers "is this value ours to update?". It does not answer "do
we have anything to update it *with*?", and the two failures look identical at
the write boundary — where `Detected<T>`, `UnknownReason` and the conflict
states all collapse into flat manifest fields. Without this amendment, a field
that P2 populated last week becomes empty this week because `git` happened not
to be on `PATH`. That is data loss wearing a sync's clothes.

**`sync` never overwrites a populated field with an absence.** If the on-disk
value is non-empty and detection returns `Detected::Unknown` for any reason, the
existing value stands. An empty field may be filled; a filled field is only ever
replaced by another *value*.

The reasons are not interchangeable, and the difference decides what the user is
told. It is the same axis split as confidence-versus-specificity: two conditions
that both block a write, only one of which the user can act on.

| Detection outcome | Write | Reported as |
|---|---|---|
| `Known`, writable | Update the field | A normal change in the plan |
| `Unknown{Conflict}` | Refused | **Actionable.** The disk disagrees with itself — two authoritative manifests claim different runtimes. Surfaced with both candidates and their evidence so the user can resolve it. |
| `Unknown{LowConfidenceOnly}` | Refused | **Actionable.** Something was found but not corroborated. Offered as a suggestion to confirm. |
| `Unknown{Unreadable}` | Refused | **Actionable.** A file is present and malformed. Names the file and the parse error. |
| `Unknown{NotAttempted}` | Refused | **Not actionable by the user, and not about their project.** `git` is missing from *this machine*. Reported once per run, not per field, and never as a property of the repository. |
| `Unknown{Timeout}` | Refused | As `NotAttempted`: a fact about this run, not about the project. |
| `Unknown{NoEvidence}` | Refused | Silent. Nothing on disk spoke to the field; there is nothing to tell the user. |

The distinction that matters most is the last three against the first three.
Reporting "could not determine the runtime" when the real cause is a missing
`git` binary invites the user to go looking at their repository for a problem
that is on their machine — the same class of error as the fabricated
`Unreadable` on a file that never existed (see §2's amendment).

### 7. v1 detector inventory

| Detector | Interest | Produces |
|---|---|---|
| `NodePackageJson` | `package.json` | runtime (`engines.node`), frameworks (deps), description, services |
| `NodeLockfile` | `pnpm-lock.yaml`, `package-lock.json`, `yarn.lock`, `bun.lockb` | runtime + framework versions at `Specificity::Lockfile` |
| `CargoToml` | `Cargo.toml` | runtime (`rust`, `rust-version`), frameworks, description |
| `PyProject` | `pyproject.toml` | runtime (`requires-python`), frameworks, description |
| `PyRequirements` | `requirements*.txt` | frameworks (`Likely` — no version pinning guarantee) |
| `GoMod` | `go.mod` | runtime (`go` directive), frameworks |
| `ComposerJson` | `composer.json` | runtime (`require.php`), frameworks, description |
| `GitRepo` | `.git` | `repo.remote`, last-commit timestamp |
| `VercelConfig` | `vercel.json`, `.vercel/project.json` | `deploy.url` (`Likely`), service `vercel` |
| `NetlifyConfig` | `netlify.toml` | service `netlify`, `deploy.url` (`Likely`) |
| `FlyConfig` | `fly.toml` | service `fly`, `deploy.url` (`Likely` — derived from app name) |
| `EnvExample` | `.env.example`, `.env.sample` | `deploy.env_required` (key names only) |

**Amendment (§7): `branch` and `dirty` are not detected.** Neither appears in
the manifest schema nor in the columns `vibe list` shows, so neither has a
consumer in v1 — and both are expensive. `git status` walks the working tree.
Every way of obtaining the branch costs either a third subprocess or a
ref-count-dependent format string: `git log --format=%D` was measured at 44ms
(0 refs), 439ms (2k refs) and 4,567ms (50k refs packed), because `%D` makes git
load and match every ref. They return the day a command displays them.

**Amendment (§2): `Specificity` is carried per `Finding`, not per `Detector`.**
One detector legitimately makes claims of differing authority — `package.json`
declaring `engines.node` is a manifest statement, while inferring a framework
from a config file's presence is not.

`DetectorSet::builtin()` is a fixed list. **No plugin loading in v1** — no dynamic libraries, no user-defined detectors. That is a real limitation (someone will want a Deno or Elixir detector on day two) and the mitigation is that the trait is small enough that a PR adding one is ~80 lines plus a fixture. A declarative, TOML-defined detector format is the v2 conversation.

### 8. Budget enforcement, and what happens when it is exceeded

The 2-second/50-repo budget is met by: one directory walk per project with aggressive pruning; `interest()`-gating so most detectors never run; a memoized read cache; `git` invocations that pass `--no-optional-locks`; and parallelism across projects with a bounded pool.

**Amendment (§8), from measurement rather than design:**

- **`std::thread::scope`, not rayon.** A fixed-size fan-out over coarse work
  items does not justify a dependency.
- **A shared work cursor, not static chunks.** Four heavy repositories among
  fifty measured a 747ms median spread across chunks and 1177ms (max 1936ms)
  clustered in one. Clustering is the *likely* arrangement, because project
  directories are scanned in path order and related projects share a prefix.
- **Two `git` invocations per repository, not one.** `remote get-url` and
  `log -1 --format=%cI`. Folding them into one is the cheapest remaining
  headroom and is deliberately not taken yet; the previous attempt to do so
  introduced a ref-count-dependent cost.
- **The practical ceiling is ~150 projects.** Cost is ~13.2ms/project and stays
  linear to at least 500, because the bottleneck is a fixed per-repository
  subprocess cost. With no `.git` present at all, 50 projects index in 34ms —
  the walking this section optimises is ~4% of the total.
- **CI cannot guard this with wall-clock.** Subprocess count per repository is
  the deterministic proxy, asserted in `tests/scan_budget.rs`.

When a detector exceeds `ctx.deadline` or a `git` call times out, its fields become `Unknown { Timeout { detector } }` and the scan report counts them. **A detector under time pressure returns nothing, never a partial guess.** `vibe scan` finishing in 1.9s with four fields honestly marked unknown is the correct outcome; finishing in 1.9s with four invented values is the failure this whole ADR exists to prevent.

## Consequences

**Easier:** A new detector is a self-contained file with a fixture directory and an `insta` snapshot of its `Vec<Finding>`. Conflicts and unknowns are first-class, displayable data with evidence attached, so `vibe show` can always answer "why do you think that?" — which is the feature that makes users trust the registry. Adding a manifest field breaks the build in every place that needs updating.

**Harder:** Every consumer of a detected value writes a `match`, including the render templates (minijinja gets `Detected` rendered via an explicit filter, not via `Display`). Detector authors must produce an `Evidence` for every value, which is genuinely more work than `Some(x)`. The read cache and `FileIndex` add a layer between detectors and the filesystem that will occasionally be in the way.

**Trade-off accepted #1:** *Polyglot repos will show an empty `stack.runtime`.* A Django backend plus a Next.js frontend in one directory produces two `Certain` runtime candidates, and step 4 refuses to choose. The user sees `runtime = ""` with a clear conflict report naming both. This is a worse first impression than guessing "node@22" and a better product — and it is softened by set-valued fields, so `frameworks` and `services` are still fully populated for those repos. If this proves too common in practice, the fix is a `[stack] runtimes` array in schema v2, **not** a tie-breaking heuristic.

**Trade-off accepted #2:** *Three discrete confidence levels cannot express fine gradations,* so a detector that is "pretty sure, but more sure than usual" has to round to `Likely` and accept manifest-writing behavior it might not want. We take the loss of expressiveness in exchange for confidence levels that actually control behavior rather than decorating output.

**Trade-off accepted #3:** *Provenance in a regenerable cache means a cache wipe downgrades `vibe sync` from "refresh my stale fields" to "propose changes I must accept."* We chose that over adding provenance keys to a manifest schema the product owner specified, and over a second sidecar file in the user's repo. The degradation is safe (fails closed) but it is a real usability cost after `rm -rf ~/.cache/vibectl`.
