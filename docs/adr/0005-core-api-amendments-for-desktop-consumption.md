# ADR-0005: Core API Amendments for Desktop Consumption

## Status

Accepted (2026-08-10). **Amends [ADR-0001](0001-crate-boundaries.md).**
**§10 rule 4 added 2026-08-10**, after implementation found that rules 1–3 —
all argv-shaped — cannot cover the positional URL slot. The old rules 4 and 5
became 5 and 6. See [ADR-0006 §9](0006-agent-management.md) for the rest of what
implementation changed: why the store's `git` calls are not `FileOp::RunCommand`,
and the per-op scoping of `SSH_AUTH_SOCK`.

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

Dropping the derive removes the *delivery mechanism* for a hostile plan. It does
not make `apply()` safe to hand a plan it did not construct, and the first draft
of this ADR wrongly implied that a `{git, gh}` program allowlist would. See
**§10** for the actual containment rules.

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

### 10. Subprocess and filesystem containment in `apply()`

An earlier draft of §1 proposed validating `RunCommand.program` against a
`{git, gh}` allowlist. **That closes nothing.** `git` invoked with arbitrary argv
*is* an arbitrary-execution primitive, and the allowlist would have been a
placeholder wearing the costume of a mitigation:

```
git -c core.sshCommand='sh -c "…"' fetch
git -c alias.x='!sh -c "…"' x
git --exec-path=/tmp/evil <anything>
git clone --upload-pack='sh -c "…"' …
```

`gh` is worse, because `gh alias set` and `gh extension install` execute arbitrary
binaries *by design* — that is the documented feature, not an abuse of it. And
argv filtering is bypassed entirely through the inherited environment:
`GIT_SSH_COMMAND`, `GIT_EXTERNAL_DIFF`, `GIT_CONFIG_COUNT` / `GIT_CONFIG_KEY_n` /
`GIT_CONFIG_VALUE_n` reach the same code paths without appearing in any argument.

The containment is therefore six rules. Rules 1–4 govern the subprocess and are
enforced where the invocation is constructed; rules 5–6 govern the filesystem and
are enforced in `apply()` at the site where preconditions are already
re-verified.

**1. The allowlist keys on the `(program, subcommand)` pair, not the program.**
Each pair carries a validated argument schema of fixed positional shape. There is
no passthrough, no user-supplied flag vector, and no variadic tail. The v1 set is
small and closed:

```rust
enum GitOp {
    Init  { cwd: PathBuf },                       // git init
    Add   { cwd: PathBuf, paths: Vec<PathBuf> },  // git add -- <paths>
    Commit{ cwd: PathBuf, message: String },      // git commit -m <msg>
    RemoteAdd { cwd: PathBuf, name: String, url: GitUrl },
    Push  { cwd: PathBuf, remote: String, branch: String },
}
```

`FileOp::RunCommand` holds one of these, not a `program` plus an `args` vector.
argv is *constructed* from the variant at apply time. This means a plan cannot
express an invocation the enum does not have a variant for, which is the same
technique as ADR-0001's missing `FileOp::Delete`: the dangerous thing is
unrepresentable rather than merely rejected.

**2. Categorical argument rejection runs before schema validation.** Any argument
matching `-c`, `--exec-path`, `--upload-pack`, `--receive-pack`, `--config-env`,
or `--namespace` is rejected outright, as is any argument beginning with `-` in a
positional slot. This is belt-and-braces given rule 1 — with constructed argv
these strings can only arrive inside a *value* (a branch name, a remote URL) —
and that is exactly why it is worth having: it catches the case where a future
variant threads a user-controlled string into a position that turns out to be
flag-parsed. A `--` separator precedes every path list.

**3. The child environment is constructed, never inherited.** `Command::env_clear()`
followed by an explicit allowlist. `PATH` and `HOME` (plus `USERPROFILE`,
`APPDATA`, `LOCALAPPDATA`, `SYSTEMROOT` and `PATHEXT` on Windows, which `gh`
needs to locate its config and which the process needs to resolve executables at
all). **No `GIT_*` or `GH_*` passthrough whatsoever.** Terminal and locale
variables are not forwarded, which also makes subprocess output stable to parse.

`GIT_CONFIG_NOSYSTEM=1` is set positively, not merely left unset. Clearing the
environment stops an *inherited* hostile value, but the system-level
`/etc/gitconfig` (or `%PROGRAMDATA%\Git\config`) is read regardless of
environment, and it can define aliases and `core.sshCommand` just as a repo-local
config can. This is defence against a machine that is already partly compromised
rather than against our own plans, which is why it is cheap to set and pointless
to argue about.

**3a. `GITHUB_TOKEN` is injected per-op, not per-environment.** The spec makes
`GITHUB_TOKEN` the fallback when `gh` is absent, but a credential in the
environment of a subprocess that has no use for it is reachable by anything that
subprocess goes on to run. So the credential is not part of the base environment:
each operation declares `fn needs_credential(&self) -> bool`, and the env builder
adds the token only for those that return `true`. In the v1 set that is
`GhOp::RepoCreate` and `GitOp::Push` — `Init`, `Add`, `Commit` and `RemoteAdd`
run with no credential in scope at all.

**Answered by [ADR-0008](0008-git-and-repository-creation.md) §4-§5
(2026-08-11): none of them, because P5 needs no credential at all.** `GhOp`
now exists and `GhOp::RepoCreate::needs_credential()` returns `false` — the op
this rule expected to be the exception is the one that demonstrates the answer. The
`gh`-present path lets `gh` own authentication; the `gh`-absent path creates no
remote and pushes nothing, reporting the exact commands that finish the job. The
API-create fallback below is designed and deliberately not built - it costs 18
net new crates including a full TLS stack, in a library every embedder links.
Rule 3a's narrowing is therefore achieved by removing the need rather than by
scoping it. The original question is kept below because the reasoning is what
justifies the answer.

**Open question as originally posed:** *which* of those ops can
actually consume a `GITHUB_TOKEN` is a P5 decision, not a P0 one. `gh` reads
`GH_TOKEN`/`GITHUB_TOKEN` from the environment and that path is straightforward.
`git push` does **not** — git has no concept of `GITHUB_TOKEN`, and the usual
bridges (a `credential.helper`, a token embedded in the remote URL) are
respectively blocked by rule 2 and unacceptable because the URL gets written into
`.git/config`. The honest possibilities are that the `gh`-absent fallback creates
the repo via the API but does not push, or that it uses an askpass helper. This
ADR fixes only the containment rule; P5 decides the mechanism, and if the answer
turns out to be "the fallback cannot push", that is a graceful degradation the
spec already permits.

**4. Any URL reaching argv is validated against a closed allowlist before
construction.** Permit `https://`, `ssh://`, the scp-like `user@host:path` form,
and an absolute local filesystem path. Reject everything else — explicitly
including any `<transport>::` form, `file://`, and any value beginning with `-`.
Rules 1–3 filter argv *shape* and cannot cover a slot whose purpose is to carry
a user string; this rule filters the value.

*Added 2026-08-10, after implementation found the hole. Rules 5 and 6 below were
numbered 4 and 5 before this was inserted; every reference has been updated. The
local-path form was folded into the rule on review the same day — it had been
recorded as an implementation widening in ADR-0006 §9a, which is one place too
far from the rule it widens.*

The diagnosis matters more than the fix. **Rules 1–3 are all argv-shaped**: they
decide which flags may appear and what the environment contains. The positional
URL slot is by definition the one place that must accept a user string, so no
argv filter can ever cover it — `--` protects the value from being *read* as a
flag and does nothing whatever about what it *means*. The rules were not wrong;
they were the wrong kind of rule for that slot.

The concrete hole, verified against git 2.54 rather than reasoned about:

```
git clone -- 'ext::touch /tmp/pwned'
```

`<transport>::<address>` makes `git` exec a remote helper named
`git-remote-<transport>`, and the built-in `ext::` helper's documented purpose is
to run an arbitrary command. It is neither a program nor a flag, so rule 1's
closed enum, rule 2's argument filter and the `--` separator all see nothing.

Two details make this worth a categorical rejection, and the second is the one a
future reader will otherwise assume was covered:

1. Modern `git` refuses `ext::` **by default**, so the naive check looks
   reassuring and a shallow test passes.
2. It is re-enabled by the **per-user** `~/.gitconfig`. **`GIT_CONFIG_NOSYSTEM=1`
   covers `/etc/gitconfig` only — while `HOME` is necessarily on rule 3's
   allowlist, because `git` cannot find its own configuration without it.** That
   is the specific hole. Under the exact constructed environment rule 3
   prescribes, with `protocol.ext.allow = always` in a user config, the command
   runs.

**The allowlist is closed, not a denylist.** A denylist is precisely what would
have missed this: nobody writes down a transport they have not heard of.

**The one non-scheme form is an absolute local path.** ADR-0006 §1 promises the
store works "against any host, or a local path, or a fork", and without a local
path the store cannot be exercised without a network. The alternative — a mocked
command runner — would leave every containment property in this section asserted
against **strings** rather than against `git`, which is the same failure as the
sabotaged guard in ADR-0002 §7: a negative control that never reaches the thing
it is controlling for. So the local path is part of rule 4, not a liberty the
implementation took against it.

Two conditions make it statable as a rule, and both are checked in
`vibe_core::url` before a `GitUrl` exists:

1. **The path must be absolute.** A relative path is resolved by `git` against a
   working directory *we* chose, so accepting one would make the meaning of a
   user-supplied string depend on our own state. `/…` and `X:\…` / `X:/…` are
   both recognised on both platforms: a Windows path arriving at a Unix build is
   a configuration error for `git` to report, not a string to reinterpret — and
   reading `C:\src\agents` as the scp-like host `C` is exactly that
   reinterpretation, turning a local path into a network fetch.
2. **The value must not begin with `-`.** This is rule 2's shape check applied at
   *construction* rather than only at use, so a bad value is reported where the
   config is read instead of from inside a clone. The `--` separator in the
   constructed argv is the second lock, not the first.

**`file://` stays rejected, and the asymmetry is not arbitrary.** A `file://` URL
goes through `git`'s URL parsing and its transport machinery; a bare absolute
path takes the local-clone path and is never parsed as a URL at all. The property
worth keeping is that the *scheme* allowlist is closed, and a value that is
unambiguously a filesystem path — absolute, with no `://` and no `::` — cannot
name a transport whatever `git` goes on to do with it.

**Cloning from a local repository does not execute that repository's hooks.**
Written down because a reader will otherwise assume it might, and reasonably so:
rule 6 below exists precisely because `.git/hooks/post-commit` is execution, and
a local clone is the one case where a *foreign* `.git/` is in reach.

> **Confirmed on git 2.54.0, 2.55.0 and 2.55.0.windows.3.** It was first
> established on 2.45.1, and that was not enough to settle it: the `ext::` hole
> above was found on 2.54, and a **negative** result about security-relevant
> behaviour established nine minor versions *behind* the one the hazard was
> found on says nothing about the newer one. New hook types, new trigger points
> and changed local-clone semantics are exactly what would invalidate it
> silently.
>
> It is now a test —
> `negative_control_cloning_a_local_repo_does_not_run_the_source_repos_hooks` —
> so it is re-established on every CI run against whatever `git` the three
> runners ship, and the versions above are the ones it has demonstrably run
> under, confirmed by name rather than inferred from a green suite. A `git`
> release that changes this makes the test red instead of making this paragraph
> quietly wrong.

The source repository was armed with every hook that could plausibly fire on a
fetch or a checkout — `post-update`, `pre-receive`, `update`, `post-receive`,
`post-checkout`, `post-commit`, `pre-push`, `proc-receive`,
`reference-transaction`, `post-index-change` — each writing a marker file.

- No marker appeared for a bare-path clone, for `file://`, or for `--no-local`.
- **The probes are known-good rather than assumed so.** An ordinary commit in
  that same repository fired the three of them a commit triggers
  (`post-commit`, `post-index-change`, `reference-transaction`), which is what
  establishes that the marker mechanism, the executable bit and the shebang all
  work. A negative result from a probe that was never exercised proves nothing,
  which is the ADR-0002 §7 failure in miniature.
- `clone` does not copy `.git/hooks`. The new clone's hooks come from the
  template directory, which is `.sample` files.
- The nearest thing to an exception is `uploadpack.packObjectsHook`, and `git`
  itself closes it: the value is **ignored when it is found in repository-level
  config**, a documented safety measure against fetching from untrusted
  repositories. Confirmed in both directions — the identical hook value fires
  when set in global config and does not fire when set in the source
  repository's own config.

What a local clone *does* spawn is `git-upload-pack` in the source repository,
which reads that repository's config. The guarantee is therefore about hooks, not
about the foreign config going unread; it holds because the one key that would
turn a fetch into an execution is the one `git` refuses to honour from there. The
global-config half of that result is not a new hole — it is the same
`~/.gitconfig` route recorded above, which `GIT_CONFIG_NOSYSTEM=1` does not
cover and which "Deliberately not taken" below declines to close.

**This rule does not belong to whichever feature hits it first.** The agent store
is the first place a user-chosen URL reaches argv and will not be the last:
`[repo] remote` in `.vibe/project.toml` is user-controlled, travels in a
*committed* file — so it arrives from repositories you clone, not only from your
own typing — and reaches argv the moment P5 wires up `gh` or `git push`. The
validator therefore lives in one place (`vibe_core::url`), and **any operation
that puts a URL in argv takes the validated type, never a `String`.** That is the
enforcement: skipping it requires changing a type signature, which is a visible
diff rather than an omission. Like the two-file window in ADR-0006 §4, this has
to be re-established for every new op that carries a URL; it does not follow
from the rule existing.

**Deliberately not taken: neutralising config-based transport re-enablement.**
Pointing `GIT_CONFIG_GLOBAL` at a null path would close the `~/.gitconfig` route
directly. It is declined because it also disables credential helpers, proxies and
`insteadOf`, which a private store or a corporate network may need — a real
functional loss for a defence that is secondary to the allowlist.

If it is ever adopted, the tension it creates must be resolved in the ADR rather
than left implicit, because "we made an exception to rule 2" is how a rule dies.
The resolution is: **rules 2 and 3 ban passthrough of user-influenced values; a
fixed set of constants we construct ourselves is a different thing.** That
distinction is already load-bearing — rule 3 sets `GIT_CONFIG_NOSYSTEM=1`
positively, which is a constructed `GIT_*` constant under a rule that otherwise
forbids all `GIT_*`. So `GIT_CONFIG_GLOBAL` would need no new principle, only the
functional trade-off above.

**4a. Every per-user config a forwarded `HOME` reaches is an execution surface,
and each command it can name must be neutralised by name.**

*Added 2026-08-11, after the third instance of one shape. Numbered `4a` rather
than inserted as a new rule 5, for the reason the renumbering note above
records.*

Rules 1–3 govern argv and the environment; rule 4 governs a value in argv. All
four are about things **we** pass. This one is about what the program does with
a file we never see:

- **`ext::` through `~/.gitconfig`.** `protocol.ext.allow = always` re-enables a
  transport whose documented purpose is running an arbitrary command.
  `GIT_CONFIG_NOSYSTEM=1` does not cover the per-user file, and `HOME` cannot be
  dropped because `git` needs it to find any configuration at all.
- **`gh`'s per-user config through the same `HOME`** (and `XDG_CONFIG_HOME`).
  ADR-0008 §6 asked whether an alias there could redirect `gh repo create` and
  verified it cannot — but the question had to be *asked*, and it was asked
  because of the first instance.
- **`gh`'s pager, through that same config.** `gh` pipes output through a
  program the config names. Not an alias, not a transport, not a URL: a third
  route to the same place, found only by enumerating what the file can invoke.

The rule is therefore not "audit the config" — a config we do not control is not
auditable — but: **before invoking a program whose config directory we must make
reachable, enumerate the ways that config can name a command, and neutralise
each one explicitly with a constructed constant.** `GH_PAGER=` (blank) is that
neutralisation for the third instance, and it is a constant we construct, which
rule 3's own carve-out already permits.

Two properties make this statable rather than a warning:

- **It is enumerable per program**, in a way "what else might a config do?" is
  not. `git`: transports, `core.sshCommand`, external diff/merge drivers,
  aliases, hooks. `gh`: aliases, extensions, the pager. That list belongs in the
  diff that adds the program, not in a later incident.
- **A neutralisation that costs functionality gets the "Deliberately not taken"
  treatment above**, not a silent omission. Disabling the pager costs nothing
  because output is piped; disabling `GIT_CONFIG_GLOBAL` outright would cost
  credential helpers and proxies, which is why it is declined in writing.

The failure mode this exists against is precise: each instance looked like a
one-off, and the third one was only found because the first two had been written
down. A fourth program — or a `gh` release that adds a config key naming a
binary — is the next instance, and the rule is what makes it a checklist item
rather than a discovery.

**4a asks one question where two are needed, and applying it correctly can
destroy the thing it protects.** *Amended 2026-08-12, from the `git check-ignore`
call site ADR-0010 §8 adds — the first instance where a key in a per-user config
is not a hazard but the answer.*

That call site has both kinds in the same file, each measured by planting a
hostile `HOME` and pairing it against an empty one:

- **`core.fsmonitor` is spawned by `git check-ignore`.** A read-only-looking
  query is an execution surface. Neutralise.
- **`core.excludesFile` changes the answer** — a control file moved from exit
  `1` to exit `0` between the two configs. It is the entire reason for asking
  `git` rather than parsing `.gitignore` ourselves, because git honours the
  redirects a project may legitimately depend on. **Neutralising it yields a
  confidently wrong answer while following this rule exactly.**

So the enumeration is two questions, not one: **which keys make the program
execute something, and which keys carry the answer we are asking the program
for.** The first set is neutralised with constructed constants; the second is
honoured, and honouring it is the point of invoking the program at all.

This is also a fourth reason to decline `GIT_CONFIG_GLOBAL`, and the first one
that is about correctness rather than cost. A neutralisation whose granularity
is *the whole config* cannot express the split above — it would take
`core.excludesFile` with it. The three reasons recorded below are functional
losses a user might choose to accept; this one makes the answer wrong.

**And this is not the fourth instance the paragraph above predicted.** It is not
a fourth program and not a new `gh` key. It is `git` — instance one — reached
through a **different subcommand**, with a key that is *not on the list this rule
wrote for git*: "transports, `core.sshCommand`, external diff/merge drivers,
aliases, hooks". `core.fsmonitor` was missing from an enumeration this document
presents as the example of the work being tractable.

The weakening is the useful part, so it is recorded rather than patched. 4a
claims the surface "is enumerable per program, in a way *what else might a config
do?* is not". Measured, the per-program list for `git` was **already incomplete
when it was written**, and nothing revealed that until a new subcommand was
invoked. **The surface is enumerable per invocation, not per program.** A list
written while adding `git clone` does not cover `git check-ignore`, so the
enumeration obligation recurs with every new `(program, subcommand)` pair rule 1
admits — the same granularity the allowlist already keys on, which is where it
should have been keyed from the start.

**5. `CreateFile` containment canonicalizes the parent, not the path.**
`canonicalize()` fails on a path that does not yet exist, which is every path a
`CreateFile` op names. So: canonicalize the deepest existing ancestor, verify
*that* is contained within a configured root or the plan's declared target
directory, and verify the remaining components contain no `..` and no absolute
prefix. Directory ops are checked the same way as they are created, so a plan
that creates `a/b/` then writes `a/b/c` re-checks at each step.

**6. No write may land under `.git/`, checked after parent canonicalization.**

Rules 1–4 close the argument vector, the environment, and the URL value. They do not close the
*repository*, and containment-to-the-project-root is not containment when `.git/`
is inside the project root:

```
CreateFile  .git/hooks/post-commit    ← passes every rule above
GitOp       Commit                    ← executes it
```

`.git/hooks/*` needs no config and no argument to run; `.git/config` supplies
aliases and `core.sshCommand` to a subsequent `git` invocation. Neither path
touches argv, so the closed `GitOp` enum is simply not in the way. The check is
therefore on the *destination*: after canonicalizing the deepest existing
ancestor, reject the op if any component of the resolved path is `.git` (or a
`.git` file pointing elsewhere, as in a worktree or submodule — resolve it and
reject the target too). `vibe` writes `.vibe/project.toml`, `README.md`,
`CLAUDE.md`, and `AGENTS.md`. It has no legitimate reason to write inside `.git/`
ever, so a categorical rejection costs exactly nothing and needs no exception
list to maintain.

**Residual risk, restated now that rule 6 exists.** The parent-canonicalization
in rule 5 is TOCTOU-vulnerable: between the check and the write, the parent can
be swapped for a symlink (Unix) or a directory junction / reparse point
(Windows), redirecting the write outside the root. Closing it properly needs
`openat`-style handle-relative I/O (`O_NOFOLLOW` /
`FILE_FLAG_OPEN_REPARSE_POINT`), which is platform-specific `unsafe`-adjacent
work v1 is not taking on.

The bound on that risk is **not** "there is no `Delete` op, so the worst case is
an additive write." That reasoning was wrong: it holds only while an additive
write is harmless, and an additive write to `.git/hooks/post-commit` is
execution. The correct statement is that rule 6 is what makes the bound true —
with the two paths that turn an additive write into execution categorically
rejected, an attacker who wins the race gets a file written somewhere
unintended, in a tree they already had write access to in order to win the race
at all. That is a real weakness and it is worth revisiting when a frontend
lands; it is not a privilege escalation.

**Why now and not P5/P6:** the same reason the `Deserialize` derive was dropped.
Rule 1 in particular changes the *shape* of `FileOp::RunCommand`, and narrowing a
public enum from `{program, args}` to a closed operation set is breaking once
anything depends on it. Capability is cheap to withhold and expensive to remove.

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
