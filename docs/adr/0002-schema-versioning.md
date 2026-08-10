# ADR-0002: Manifest Schema Versioning and Forward Compatibility

## Status

Accepted (2026-08-10). **Revised (2026-08-10)** by product-owner decision, before
any implementation: `schema_version` changed from a single integer to
`major.minor`, and the archive semantics in §6 were completed. §1, §2, §3, §6 and
§7 are the revised text; the rest stands as originally accepted.

## Context

`.vibe/project.toml` is user-owned data. It lives inside the user's repo, it will
be committed, it will be hand-edited, and it will be read by different `vibectl`
versions on a laptop, a desktop, a CI runner, and eventually a Tauri app — often
out of sync with each other. It will also be edited by AI agents that were handed
a rendered `CLAUDE.md`.

That gives us three failure modes to design against, in descending order of
severity:

1. **`vibe sync` silently drops a key it did not understand.** This is data loss
   in a file the user has committed. It is the failure we must make impossible.
2. **A newer `vibectl` writes something an older one then misreads and
   "corrects."** Two machines fight over a file.
3. **A hand-written or pre-versioning manifest is rejected outright,** turning a
   registry that is supposed to embrace an existing mess into one that demands
   conformance first.

`toml_edit` (rather than `toml`) is already mandated for writes, which gives us
the machinery to preserve comments and formatting — but `toml_edit` only
preserves what you don't overwrite. Preservation is an API-design property, not a
free property of the crate.

## Decision

### 1. The field: `major.minor` as a string, first line of the file

```toml
schema_version = "1.0"

[project]
name = "macroring"
...
```

**Top-level, not inside `[project]`.** It is metadata about the *file*, not a
property of the project. Putting it in `[project]` would mean it appears in the
domain model, in `vibe show` output, and in every render template, where it does
not belong. A bare key before any table header is also unambiguously first in
TOML, so a short prefix read can determine readability before committing to a
full parse.

**A quoted string, not a TOML float.** `schema_version = 1.10` would parse as
`1.1` and silently order *before* `1.9`. A string parsed into `(u16, u16)` has no
such trap and is what a hand-editing human types.

**Two components, not one.** An earlier draft of this ADR used a single integer on
the theory that the only question is "can this build read this file", which wants
a total order. That was wrong, and the reason is §2: it conflated *additive* and
*breaking* change into one number, which forced every additive field to either
bump nothing (leaving no way to describe the file) or break every older binary
in the wild. Two components let the compatibility rule be read directly off the
version.

**Patch is deliberately absent.** There is no third component because there is no
third class of change: a manifest edit either preserves the meaning of existing
keys or it does not.

### 2. The bump rule

- **Minor bump — additive only.** A new optional key or a new table. Older builds
  read the file correctly, ignore what they do not recognize, and preserve it on
  write (§5).
- **Major bump — any change to the meaning of an existing key.** A rename, a
  removal, a type change, a units change, or a change in how a value is
  interpreted. An older build cannot read such a file correctly and must not try.

Stated as one sentence: **minor means "there is more here than you know about",
major means "what you think you know is wrong."**

### 3. Reading a manifest with a version this build does not fully support

| Case | Behaviour |
| --- | --- |
| `major` > supported major | **Refuse the manifest.** `CoreError::SchemaMajorMismatch`. |
| `major` < supported major | Migrate in memory (§4). |
| `major` equal, `minor` > supported minor | **Proceed.** Ignore unrecognized keys, preserve them on write, emit one warning. |
| `major` equal, `minor` <= supported minor | Normal path. |

**On a major mismatch, the *manifest* is refused — not the whole command.** This
distinction is load-bearing and is an interpretation of the decision worth
stating explicitly, because "refuse on every command that reads a manifest" could
be read either way. Aborting `vibe list` across 30 projects because one of them
has a manifest from a future build would break the registry premise in exactly
the multi-machine situation the tool exists for. So:

- `vibe list` renders that project as an error row (name and path from the cache
  or the directory, every derived column replaced with an error marker) and
  continues. The process exit code reflects that something was unreadable.
- `vibe show <that project>` fails with `SchemaMajorMismatch`.
- Every command that would write to it — `sync`, `archive`, `render` — fails with
  the same error.
- `--json` carries the error object in that project's slot, so a script sees a
  structured failure rather than a missing entry.

**Exit codes.** "Something was unreadable" must be distinguishable from "the
command failed", or a script cannot tell a partially-read registry from a broken
one. The full table, owned by `vibectl/src/exit.rs`:

| Code | Meaning | Emitted by |
| --- | --- | --- |
| `0` | Success. Every requested project was read and every requested write applied. | any command |
| `1` | Failure. The command produced no useful result — bad arguments, no registry, an unreadable cache that could not be rebuilt, or a write that was refused or aborted. | any command |
| `2` | **Partial.** Results were produced, but at least one entry could not be read. | read commands only |

`2` is reachable only from `list`, `show` and `scan`. Write commands never exit
`2`: per ADR-0001 §3 a `WritePlan` is all-or-nothing at the decision level, so a
write either applied or did not. A major-schema mismatch on the *target* of a
write is therefore a `1`, while the same manifest encountered while listing 30
projects is a `2`.

`2` is not exclusive to schema mismatches — an unreadable directory or a manifest
with a TOML syntax error produces it too. The rule is about the shape of the
outcome, not its cause.

**Error text names both numbers and the action.** The `Display` impl produces:

```
manifest uses schema 3.0, this build reads 2.x — upgrade vibe
```

`CoreError` carries `found: (u16, u16)` and `supported_major: u16` as data; the
sentence above is `vibectl`'s rendering of it, per ADR-0001 §4.

**The minor-newer warning is emitted once per run, not once per manifest.** Core
emits a `Diagnostic { code: "VIBE_W_SCHEMA_MINOR_NEWER", .. }` per affected
manifest, because core does not know what a "run" is; `vibectl` coalesces them
into a single stderr line naming the count. A `vibe scan` over 30 forward-versioned
manifests must not print 30 warnings.

**There is no override flag.** The earlier draft had `--allow-newer-schema`; it is
removed. Under the revised rule the case it existed for — an additive field in a
newer manifest — now proceeds by default, and the remaining case (major mismatch)
is precisely the one where the build cannot know what it misparsed. In
particular there is **no `--force` on `render`**: rendered `CLAUDE.md` and
`AGENTS.md` are fed into an AI agent's context, and a half-understood manifest
propagated into an agent's context is worse than a refusal. If it renders, it
renders clean.

### 4. Reading an OLDER manifest, or one with no version at all

- **Missing `schema_version` is treated as `1.0`, not as an error.** Hand-written
  manifests and manifests from the pre-versioning prototype must work. Being
  strict here would punish exactly the users this product targets.
- **Older majors migrate in memory, on every read, always.** Migration is a chain
  of pure functions `migrate_v1_to_v2(&mut DocumentMut) -> Result<(), CoreError>`.
  They operate on the `toml_edit::DocumentMut`, **not** on the typed `Manifest` —
  a migration must carry comments and formatting forward too, and a typed struct
  has already thrown those away. An older *minor* needs no migration by
  construction, since minor changes are additive.
- **In-memory migration is never written back implicitly.** A read command leaves
  the file byte-identical. Migration reaches disk only as part of an explicit
  write command, and appears in the `WritePlan` as its own visible `UpdateFile`
  op with `EditReason::SchemaMigration { from, to }`, so `vibe sync --dry-run`
  shows the user their file is about to be upgraded before it happens.

### 5. Unknown-key survival: the typed struct is not the write source

**`Manifest` is a read projection with no way to become TOML.** It does not
implement `Display`. It has no `to_toml()`. Its `Serialize` impl exists only for
`--json`/IPC and is not wired to any TOML serializer. There is no code path from
`Manifest` back to a `.vibe/project.toml`.

Writes go through the only type that can produce TOML:

```rust
// vibe-core/src/manifest/document.rs
pub struct ManifestDocument { path: PathBuf, doc: toml_edit::DocumentMut, original: String }

impl ManifestDocument {
    pub fn open(path: &Path) -> Result<Self, CoreError>;
    pub fn parse(&self) -> Result<Manifest, CoreError>;         // read projection
    pub fn apply(&mut self, edit: FieldEdit) -> Result<(), CoreError>;  // targeted mutation
    pub fn render(&self) -> String;                              // the loaded doc, re-serialized
    pub fn into_op(self) -> Option<FileOp>;                      // None if render() == original
}

/// The complete, closed set of mutations any command may perform.
pub enum FieldEdit {
    SetProjectStatus(Status),
    SetArchived(bool),
    SetDescription(String),
    SetStackRuntime(Detected<String>),
    ReplaceStackFrameworks(Vec<String>),
    ReplaceStackServices(Vec<String>),
    SetRepoRemote(Detected<String>),
    SetRepoVisibility(Visibility),
    SetDeployUrl(Detected<String>),
    ReplaceDeployEnvRequired(Vec<String>),
    SetSchemaVersion { major: u16, minor: u16 },
}
```

Every edit is a targeted mutation of one addressed path in the live `DocumentMut`
(`doc["stack"]["frameworks"]`). Keys nobody addressed are untouched by
construction, because nothing ever rebuilds the document. Comments, key order,
whitespace, and inline-vs-multiline array style survive because they are still
the same `toml_edit` document that was read off disk.

**`vibe new` is the sole exception,** and it is not really one: it creates a file
that does not exist, from a template, so there is nothing to preserve.

Two supporting rules:

- **`Manifest` records what it did not understand,** as
  `unknown: Vec<UnknownKey { dotted_path, kind }>`. This is used *only* for
  reporting — `vibe show` can say "3 keys this build does not understand:
  `deploy.regions`, `context.risks`, `ai.summary`" — never for round-tripping.
  This is also what makes the minor-newer path in §3 safe: the keys are visible
  to the user and preserved on disk, not silently swallowed.

  **An unknown *table* is reported as one entry and is not recursed into.** A
  future `[ai]` section reports as `ai`, not as `ai.model`, `ai.summary`,
  `ai.generated_at`. The user can act on "this build does not understand `ai`"
  — upgrade, or ignore it — and cannot act on a list of sub-keys belonging to a
  feature this build does not have. The named case to revisit is `vibe show`:
  if it ever grows a mode that dumps a manifest for a human to audit key by
  key, per-key detail becomes useful there and this decision should be
  reopened rather than rediscovered. Preservation on write is unaffected
  either way — it comes from never rebuilding the document, not from this
  list.
- **Unknown *values* degrade, they do not fail.** A `status = "hibernating"`
  written by a future build parses as `Status::Other("hibernating".into())`,
  displays verbatim, and is never rewritten. Only an explicit `vibe
  archive`/status change replaces it. The same rule applies to unrecognized
  entries in `stack.services` and friends: they are strings, they are kept.

### 6. `archived` is orthogonal to `status`

```toml
[project]
status = "shipped"
archived = true
```

- **`vibe archive` sets `[project] archived = true` and does not touch
  `status`.** Un-archiving is therefore not a guess: `archived = false` restores
  the exact prior state because the prior state was never overwritten.
- **`vibe unarchive <name>` is part of the v1 CLI surface.** The "un-archiving is
  not a guess" property is the entire justification for a separate key, and
  without an inverse command it is a property nobody can reach. It is one
  `FieldEdit::SetArchived(false)`, the same op `archive` uses with the other
  operand, so it costs a clap subcommand and nothing else. `archive` on an
  already-archived project and `unarchive` on a non-archived one are both
  no-ops that produce an empty `WritePlan` (`ManifestDocument::into_op` returns
  `None` when the rendered document is unchanged), not errors.
- **`status` and `archived` are two dimensions and every combination is legal.**
  `dead` narrows to *abandoned before completion*; `archived` means *off my
  desk*. `shipped + archived` (finished, put away) and `dead + archived`
  (abandoned, put away) are different states and both must round-trip through
  `open → parse → render` unchanged.
- **`vibe list` hides archived projects by default; `--all` includes them.**
  `--json` always includes the `archived` field regardless, because a machine
  consumer filters for itself and a field that appears conditionally is a field
  that breaks scripts.
- **Absent `archived` parses as `false`.** Existing manifests are unaffected, so
  this is a minor-version change per §2, not a major one.

`created` is an ISO-8601 date string (`YYYY-MM-DD`), parsed leniently. TOML has a
native date type; we deliberately do not use it, because half these manifests
will be hand-written or agent-written and a quoted string is what everyone
actually types. An unparseable `created` becomes `Detected::Unknown`, not an
error.

### 7. Constants and testing

```rust
pub const SCHEMA_MAJOR: u16 = 1;
pub const SCHEMA_MINOR: u16 = 0;
```

Only the major is compared for the refuse/accept decision; the minor is compared
only to decide whether to warn.

### The rule for testing a preservation property

Written down because intending to get it right has now failed three times, and
been caught by the same mechanism all three times.

**Any test asserting that something is preserved must (a) place its sentinels on
keys the command under test actually writes, and (b) be negative-controlled by
breaking the preservation mechanism and observing the test fail.**

Sentinels only on untouched keys test that the parser does not corrupt the file.
They do not test what the design exists to guarantee — that the *edited* key
keeps its surroundings. Three times a preservation test was written that passed
against an implementation with decor preservation deliberately removed:

- P1, the `toml_edit` round-trip property: every sentinel was on an unedited key.
- P3, `sync` at the command level: the same omission, in a test written
  specifically because of the first one.

In both cases the test was correct-looking, passed, and proved nothing. In both
cases the negative control found it in one run. **The negative control is not a
nice-to-have here; it is the only thing that has ever detected this.**

**A negative control must demonstrate that the sabotaged guard was *reached*.**
A test that fails before the guard executes proves nothing about the guard.
Build the fixture so the operation succeeds to completion when the guard is
removed.

Added after the fourth appearance of this family, and the sharpest one. The
first three were *weak assertions* — sentinels on keys nothing wrote. The fourth
was different and worse: `agents update` refusing a store directory that is not
ours. The assertion was correct, the guard was sabotaged, and the test still
failed — but it failed because the fixture's `git fetch` errored out on its own,
several steps *before* the sabotaged check would have mattered. The test proved
nothing while looking exactly like proof, and would have kept passing if the
guard had been deleted outright.

Rebuilding the fixture so every step downstream of the check succeeds — giving
the victim repository a working `origin` of its own — showed the sabotaged
build running `reset --hard` on a user's repository to completion. That is what
the control is for, and the distinction between the two fixtures is the whole
lesson: **a negative control is an experiment, and an experiment that terminates
early has not been run.**

**A negative control must also fire *deterministically*.** One whose firing
depends on winning a race can stop proving anything without ever failing, which
is the same outcome as the guard never being reached, arrived at from the other
direction — and worse, because it arrives as a green check.

Added after the fifth appearance, found by review rather than by a failure. The
`ext::` control in `agents_store.rs` pointed the remote helper at
`touch <marker>` and asserted the marker existed. But `touch` is a child `git`
spawns and then kills the moment the helper fails to speak the protocol, so the
assertion was really *"`touch` won that race"*. On a loaded runner it need not,
and the test would then report **pass** — silently returning `GitUrl::parse`'s
`::` rejection to a guard against a hazard nobody had demonstrated.

The repair is the general one: assert on something the system under test does
**synchronously and reports itself**, not on a side effect that has to win.
Naming a program that cannot exist (`ext::vibe-nonexistent-helper-probe`) makes
`git` print what it *tried to spawn* — the execution primitive itself — with
nothing needing to run or exist. Where an assertion genuinely cannot be made
deterministic, it must **fail loudly when it could not run**, never pass quietly.

The corollary, from `W_SCHEMA_MINOR_NEWER`: a warning defined and never emitted
is a policy that exists only in this document. A test asserting a diagnostic is
reported must check that something *produces* it, not merely that a consumer
would render it if given one.

The regression test that matters, and that must exist before the first write
feature ships: a corpus of manifests in `crates/vibe-core/tests/corpus/` — each
with comments, blank lines, mixed array styles, and keys from a hypothetical
future minor — round-tripped through `open → apply(every FieldEdit variant) →
render`, snapshotted with `insta`. A dropped comment or a vanished unknown key
becomes a failing snapshot diff, which is the only way this invariant stays true
after the tenth feature. The corpus must include a `shipped + archived` and a
`dead + archived` manifest, per §6.

## Consequences

**Easier:** `vibe sync` is safe to run on any manifest, including ones from
builds that do not exist yet. Users can hand-edit freely and add their own keys;
the tool becomes a good citizen in a file it does not exclusively own.
Multi-machine drift resolves by upgrading `vibectl`, never by losing data.
Additive schema growth — which is the overwhelmingly common case — no longer
breaks any deployed binary.

**Harder:** Every new manifest field requires a `FieldEdit` variant and a
corresponding arm in the document writer — more ceremony than `serde`
round-tripping. The `Manifest`/`ManifestDocument` split means two types where a
naive design has one, and contributors will reach for the wrong one until the API
stops them (it will: `Manifest` cannot produce TOML). Two version components mean
every schema change now requires a judgement about which one moves, and getting
that judgement wrong is what breaks compatibility rather than the change itself.

**Trade-off accepted #1:** *a major mismatch makes a project unreadable rather
than partially readable.* We no longer attempt a best-effort read across a major
boundary. This is a deliberate reversal of the earlier draft: a best-effort read
of a file whose key meanings changed produces confidently-wrong output, which for
a tool whose fifth constraint is "detection is honest" is the worse failure. The
mitigation is that the project still appears in `vibe list` as an error row, so
the registry never silently loses an entry.

**Trade-off accepted #2:** *a `vibe sync` on an old manifest upgrades it in place,
after which an older `vibectl` on another machine can read it but no longer write
it.* Migration is one-way and there is no downgrade path. Migrations are rare by
construction (§2), and we mitigate this by making the migration a visible line in
`--dry-run` output rather than an invisible side effect of an unrelated command.

**Trade-off accepted #3:** *two orthogonal fields mean two things to keep
consistent.* `status = "dead"` with `archived = false` is a legal state that reads
oddly ("abandoned but still on my desk"). We accept it rather than adding
validation, because the alternative — inferring one field from the other — is
exactly the information-destroying coupling §6 exists to avoid.
