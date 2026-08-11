# ADR-0007: Rendered Files — Ownership, the Marker, and Why It Carries a Hash

## Status

Proposed (2026-08-11). Design for P4 (`vibe render`).

## Context

`vibe render` generates `CLAUDE.md`, `AGENTS.md` and `README.md` from the
manifest. Every one of those lands in a directory the user owns, and one of them
— `README.md` — is routinely the most valuable prose file in a repository.

ADR-0006 already drew the line this ADR sits on the other side of:

> **Installed agents are not rendered files.** […] A rendered `CLAUDE.md` (P4)
> carries an overwrite marker and is regenerated from the manifest; its content
> is *derived*, and clobbering it loses nothing. An installed agent is a
> user-editable artifact whose whole point is that you can edit it. **The
> mechanisms must not be shared.**

That sentence assumed a marker existed and did not say what it was. This ADR
says. The mechanisms stay unshared; the *ownership question* is the same one,
and it gets a different answer for a stated reason.

## Decision

### 1. Whole-file ownership, proved by a marker in the file itself

A rendered file is **entirely** `vibe`'s. There are no user-editable regions
inside it, and nothing outside it is touched.

Region markers (`<!-- vibe:begin -->` … `<!-- vibe:end -->`) were considered and
rejected. They would make `README.md` safe by construction, and they cost a
block-location problem: what happens when the block is deleted, duplicated,
nested, or moved inside a fenced code block that a renderer must not treat as a
marker. Every one of those is a state needing a defined answer, which is the
ten-state table of ADR-0006 §5 again for a feature whose whole output is
derivable. Whole-file ownership has three states, and they are the interesting
ones.

### 2. The marker carries the hash, so there is no third on-disk format

```markdown
<!-- vibe:generated v1 hash=b3:1f9a…c4 -->
```

**The marker is both the ownership proof and the integrity record.** Ownership
is "this file begins with our marker". Integrity is "the rest of the file hashes
to what the marker says".

The alternative — recording rendered-file hashes in `.vibe/agents.lock`, or in a
new `.vibe/render.lock` — was rejected for the reason ADR-0006 §4 rejected a
journal: it buys exactness at the cost of a third on-disk format with its own
version gate, its own corruption story, and its own "what if it disagrees with
the file" table. A self-describing file needs none of that. It also survives
being copied, moved, or restored from a backup, which a sidecar record does not.

`hash=` reuses the `b3:` algorithm prefix and the `NotAttempted`-versus-`Conflict`
discipline from ADR-0006 §3: an unrecognised algorithm is `Unverifiable`
("cannot tell"), never `Modified` ("differs"). A build that changes the hash
algorithm must not report every rendered file in the world as edited.

`v1` is the **marker** format version, not the schema version and not the
template version. It exists so the marker's own shape can change without every
prior rendered file becoming unreadable.

### 3. The hash is computed over normalised bytes

Line endings are normalised (`\r\n` → `\n`) and a single trailing newline is
enforced **before** hashing, and the marker line itself is excluded.

This is not tidiness. `git` converts line endings on checkout under
`core.autocrlf`, which is the default on Windows. A hash taken over the bytes as
written would break on the next clone, and **every rendered file in the
repository would report as modified** — offering to overwrite work nobody
touched, on a machine that did nothing wrong. That is the same failure mode the
algorithm prefix exists to prevent, arriving by a different route.

### 4. Three states, and `render` refuses two of them

| State | Detected by | `render` | `render --force` |
| --- | --- | --- | --- |
| **Absent** | No file | writes it | writes it |
| **Generated** | Marker present, hash matches | overwrites | overwrites |
| **Modified** | Marker present, hash differs | **refuses** | overwrites |
| **Foreign** | File present, no marker | **refuses** | **still refuses** |
| **Unverifiable** | Marker present, unknown hash algorithm | **refuses** | overwrites |

**`--force` does not override `Foreign`, and that is the whole point.** `--force`
is the user saying "yes, discard my edits to *your* file". It is not a way to
claim a file `vibe` never wrote. A hand-written `README.md` is `Foreign` and stays
`Foreign` no matter what flag is passed — the same rule as ADR-0006 §5's
`PresentUnowned`, which `--force` also does not override.

This is what makes `README.md` safe to have as a target. The dangerous case is
not "the user edited our README", it is "the user has a README and we never
wrote it", and that case is refused categorically rather than flagged.

### 5. `Modified` refuses, unlike a cache and like an agent

ADR-0006 says clobbering a rendered file "loses nothing" because its content is
derived. That is true of the *content* and false of the *edit*. If a person
opened `CLAUDE.md` and added a paragraph, the paragraph is not derivable from
the manifest, and regenerating destroys it.

So `Modified` refuses without `--force`, exactly as an edited agent does. The
mechanisms are unshared — a marker in the file versus a lockfile entry — but
they arrive at the same rule, because the rule follows from "never destructive"
rather than from either mechanism.

The honest consequence, stated rather than buried: **`render` is not idempotent
across a manual edit.** A user who edits a generated file must either revert it
or pass `--force`; there is no merge. Merging would mean interpreting the
user's prose, which is the same refusal as ADR-0006's trade-off #3.

### 6. `render` writes through `WritePlan` like everything else

No new write path. `plan_render` returns a `WritePlan` of `CreateFile` /
`UpdateFile` ops, so `--dry-run` shows the whole generated file before it lands,
containment rules 5–6 apply, and the diff is real because `UpdateFile` carries
both sides.

### 7. Templates are compiled in, with no filesystem loader

`minijinja` with `include_str!` templates and **no filesystem loader
configured**. A loader would make template resolution depend on the working
directory, which turns "render this project" into "render this project with
whatever templates happen to be next to it" — a code-execution-adjacent surprise
in a tool that already goes to some length to make its subprocess behaviour
independent of ambient state. User-supplied templates are a v2 conversation and
would need their own containment answer.

## Consequences

**Easier:** a rendered file is self-describing. Ownership and integrity travel
with it, so a clone, a copy or a restored backup answers "is this ours, and has
it changed" with no sidecar and no version gate.

**Harder:** the marker is visible in the user's file, and it must survive their
editor. A hash in a comment invites the question "can I just edit that", to
which the answer is yes and it is forging rather than a bug.

**Trade-off accepted #1:** *`README.md` can only ever be generated into a
project that does not already have one.* For most real repositories `vibe render
readme` will refuse forever. That is the correct outcome and not a limitation to
engineer around.

**Trade-off accepted #2:** *the marker's hash can be edited to match, defeating
the check.* A user who does that has explicitly claimed the file is unmodified.
The check exists to prevent accidents, not to resist the file's owner.
