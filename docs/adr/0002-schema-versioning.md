# ADR-0002: Manifest Schema Versioning and Forward Compatibility

## Status

Accepted (2026-08-10)

## Context

`.vibe/project.toml` is user-owned data. It lives inside the user's repo, it will be committed, it will be hand-edited, and it will be read by different `vibectl` versions on a laptop, a desktop, a CI runner, and eventually a Tauri app — often out of sync with each other. It will also be edited by AI agents that were handed a rendered `CLAUDE.md`.

That gives us three failure modes to design against, in descending order of severity:

1. **`vibe sync` silently drops a key it did not understand.** This is data loss in a file the user has committed. It is the failure we must make impossible.
2. **A newer `vibectl` writes something an older one then misreads and "corrects."** Two machines fight over a file.
3. **A hand-written or pre-versioning manifest is rejected outright,** turning a registry that is supposed to embrace an existing mess into one that demands conformance first.

The product owner's schema has no version field. `toml_edit` (rather than `toml`) is already mandated for writes, which gives us the machinery to preserve comments and formatting — but `toml_edit` only preserves what you don't overwrite. Preservation is an API-design property, not a free property of the crate.

## Decision

### 1. The field: a top-level integer, first line of the file

```toml
schema_version = 1

[project]
name = "macroring"
...
```

**Top-level, not inside `[project]`.** It is metadata about the *file*, not a property of the project. Putting it in `[project]` would mean it appears in the domain model, in `vibe show` output, and in every render template, where it does not belong. A bare key before any table header is also unambiguously first in TOML, so a 200-byte prefix read can determine readability before committing to a full parse.

**A single monotonic integer, not semver.** The manifest needs exactly one question answered — "can this build safely read and write this file?" — and that question wants a total order, not three numbers where two are noise. `1`, then `2`, then `3`.

### 2. The bump rule: additive changes never bump

This is the load-bearing half of the integer decision.

- **Adding a new optional key or a new table does NOT bump `schema_version`.** New keys are handled entirely by the unknown-key preservation rule (§5). A v1 manifest written by a future `vibectl` with three new keys is still a valid v1 manifest.
- **`schema_version` bumps if and only if the *meaning* of an existing key changes** — a rename, a removal, a type change, a units change, or a change in how a value is interpreted.

Consequently version bumps are rare and are *always* breaking. There is no such thing as a "minor" manifest version, which is why semver would have been three fields to express one bit.

### 3. Reading a NEWER manifest: read-only, degraded, loudly

When `schema_version > SUPPORTED_SCHEMA_VERSION`:

- **Read commands work.** `vibe list`, `vibe show`, and `--json` parse the file best-effort and display what they recognize. Refusing to *list* a project because its manifest is from next month's build would make the registry useless in exactly the multi-machine scenario it exists to serve.
- **Every command that produces a file is refused.** `sync`, `archive`, `render`, and `new`-into-an-existing-dir all fail with `CoreError::SchemaTooNew`. This includes `render`, even though `render` only writes a *different* file: rendered `CLAUDE.md` / `AGENTS.md` are fed to AI agents, and propagating a misread semantic into an agent's context is worse than not rendering.

  The rule, stated once: **a newer manifest is inspectable, never actionable.**
- Degraded reads are marked, not hidden. `ProjectView` carries `compat: Compat::NewerSchema { found, supported }`; `--json` output includes both `schema_version` and `compat` at the top level of every project object; the human table marks the row. A script consuming `--json` can therefore tell that it is looking at a partial read without parsing prose.
- Escape hatch: `--allow-newer-schema` downgrades the refusal to a warning for the user who knows what they are doing. It is still bound by §5, so even the escape hatch cannot clobber an unknown key.

### 4. Reading an OLDER manifest, or one with no version at all

- **Missing `schema_version` is treated as `1`, not as an error.** Hand-written manifests and manifests from the pre-versioning prototype must work. Being strict here would punish exactly the users this product targets.
- **Older versions migrate in memory, on every read, always.** Migration is a chain of pure functions `migrate_v1_to_v2(&mut DocumentMut) -> Result<(), CoreError>`, applied in sequence. They operate on the `toml_edit::DocumentMut`, **not** on the typed `Manifest` — because a migration must carry comments and formatting forward too, and a typed struct has already thrown those away.
- **In-memory migration is never written back implicitly.** A read command leaves the file byte-identical. Migration reaches disk only as part of an explicit write command, and appears in the `WritePlan` as its own visible `UpdateFile` op with `EditReason::SchemaMigration { from, to }`, so `vibe sync --dry-run` shows the user their file is about to be upgraded before it happens.

### 5. Unknown-key survival: the typed struct is not the write source

The mechanism, and the reason it cannot be forgotten:

**`Manifest` is a read projection with no way to become TOML.** It does not implement `Display`. It has no `to_toml()`. Its `Serialize` impl exists only for `--json`/IPC and is not wired to any TOML serializer. There is no code path from `Manifest` back to a `.vibe/project.toml`.

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
    SetSchemaVersion(u32),
}
```

Every edit is a targeted mutation of one addressed path in the live `DocumentMut` (`doc["stack"]["frameworks"]`). Keys nobody addressed are untouched by construction, because nothing ever rebuilds the document. Comments, key order, whitespace, and inline-vs-multiline array style survive because they are still the same `toml_edit` document that was read off disk.

**`vibe new` is the sole exception,** and it is not really one: it creates a file that does not exist, from a template, so there is nothing to preserve.

Two supporting rules:

- **`Manifest` records what it did not understand,** as `unknown: Vec<UnknownKey { dotted_path, kind }>`. This is used *only* for reporting — `vibe show` can say "3 keys this build does not understand: `deploy.regions`, `context.risks`, `ai.summary`" — never for round-tripping. Reporting them is how we keep the user informed without pretending we can act on them.
- **Unknown *values* degrade, they do not fail.** A `status = "hibernating"` written by a future build parses as `Status::Other("hibernating".into())`, displays verbatim, and is never rewritten. Only an explicit `vibe archive`/status change replaces it. The same rule applies to unrecognized entries in `stack.services` and friends: they are strings, they are kept.

### 6. Two schema points the owner's example left open

- **`archive` gets its own key: `[project] archived = true`,** not a new `status` value. `status` is a lifecycle judgement (`idea | active | paused | shipped | dead`) and archival is an orthogonal shelving action — a *shipped* project can be archived without becoming *dead*. Overloading `status` would destroy information on archive and make un-archiving a guess. `archived` is a new optional key, so per §2 it does not bump `schema_version`.
- **`created` is an ISO-8601 date string (`YYYY-MM-DD`), parsed leniently.** TOML has a native date type; we deliberately do not use it, because half these manifests will be hand-written or agent-written and a quoted string is what everyone actually types. An unparseable `created` becomes `Detected::Unknown`, not an error.

### 7. Constants and testing

```rust
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;
pub const MIN_READABLE_SCHEMA_VERSION: u32 = 1;
```

The regression test that matters, and that must exist before the first write feature ships: a corpus of manifests in `crates/vibe-core/tests/corpus/` — each with comments, blank lines, mixed array styles, and keys from a hypothetical v99 — round-tripped through `open → apply(every FieldEdit variant) → render`, snapshotted with `insta`. A dropped comment or a vanished unknown key becomes a failing snapshot diff, which is the only way this invariant stays true after the tenth feature.

## Consequences

**Easier:** `vibe sync` is safe to run on any manifest, including ones from builds that do not exist yet. Users can hand-edit freely and add their own keys; the tool becomes a good citizen in a file it does not exclusively own. Multi-machine drift resolves by upgrading `vibectl`, never by losing data.

**Harder:** Every new manifest field requires a `FieldEdit` variant and a corresponding arm in the document writer — more ceremony than `serde` round-tripping. The `Manifest`/`ManifestDocument` split means two types where a naive design has one, and contributors will reach for the wrong one until the API stops them (it will: `Manifest` cannot produce TOML).

**Trade-off accepted #1:** *A `vibe list` on a newer manifest can display a semantically wrong value.* We chose best-effort reads over refusal. If v2 redefines `stack.runtime` from `"node@22"` to a table, a v1 build shows garbage or nothing for that row. We accept this because the alternative — hiding the project entirely — breaks the tool's core promise in the exact situation (mid-upgrade, multiple machines) where the user most needs their inventory. The mitigation is that the degradation is *labelled* in both human and JSON output, so it is never silent.

**Trade-off accepted #2:** *A `vibe sync` on an old manifest upgrades it in place, after which an older `vibectl` on another machine can read it but no longer write it.* Migration is one-way and there is no downgrade path. We accept this because migrations are rare by construction (§2), and we mitigate it by making the migration a visible line in `--dry-run` output rather than an invisible side effect of an unrelated command.

**Trade-off accepted #3:** *`vibe render` is refused on a newer manifest even though rendering writes a different file.* This is stricter than strictly necessary and will annoy someone. We are buying a single memorable rule ("newer = inspect only") over a nuanced per-command matrix, and avoiding the worse failure of feeding a misread manifest into an AI agent's context window.
