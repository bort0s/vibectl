# ADR-0011: Live agent monitoring — agents report, and the last event is never terminal

## Status

**Scoped (2026-08-12), design open, no code.** P6's second feature. This records
what was measured, the one architectural decision taken in principle, and the
constraints that hold *whatever* the design turns out to be — the shape ADR-0009
used for the frontend, and for the same reason: "nothing recorded" and "no
constraints" are different things.

**Re-pinned to Claude Code 2.1.233 (2026-08-17), and the previous corpus is
deliberately not re-measured.** The build every fact below was first established
on — **2.1.228** — no longer exists on this machine; 2.1.229 and 2.1.233 are
installed in its place. A second round was taken on 2.1.233 and is recorded as
**deltas against the 2.1.228 record**, not as a replacement for it. Re-measuring
the whole corpus was considered and declined: it is expensive and it produces
another corpus that goes stale by the same mechanism on the next release. **The
value is in the delta**, which is what a reader of a versioned measurement
actually needs.

**That round retracted a claim rather than adding one, and the retraction is the
headline.** §5's sentence that an unmatched `tool_use_id` means a tool is in
flight is **false on 2.1.233 and nothing replaces it.** The gap §5 claimed to
have narrowed is open. §8's open items are revised accordingly.

**And transport — missing from §8 entirely, which read as complete — is decided
in §7a, by a decision that was taken and then reversed.** `http` was chosen on a
property no other variant has, costed, and the costing showed the property to be
information about the receiver rather than about the subject. The transport is a
file. **Both halves are kept**, because the reversal is the more useful record: a
real and unique property is not the same as a decisive one, and this one did not
survive being priced.

**This round is also the first measured instance of a staleness channel this
project had only predicted.** ADR-0005 §10 rule 4a records that upstream releases
are the one channel with no trigger — no diff on our side, nothing to review. It
happened: a vendor bumped a version and an ADR claim became false with no change
to this repository. Recorded there rather than here, because the next person
relying on a measured property of someone else's tool will be reading the rule.

**Round 3 (2026-08-19) is on 2.1.234, and it is again a delta.** The channel
predicted in ADR-0005 §10 rule 4a fired a second time and 2.1.233 is gone. Two
builds were installed side by side — 2.1.234 and 2.1.235 — and **the session was
running the older one**, so this round records the binary's *path* alongside its
version; a version alone would not have said which of the two produced the
numbers. Three findings, all deltas: **ten event types fired where round 2
recorded nine** (§2), **the `command` hook carries ten properties where §7a named
three** (§2, §7a, §9), and **the intra-agent-concurrency limit's precondition
turned out to be constructible** — six parallel tool calls from one agent — while
the hazard it guards did not appear (§7a). Nothing was retracted this round; two
enumerations were widened and one *"may never be constructible"* was withdrawn.

**Round 3b (2026-08-19), same build, and it withdraws two more claims — this
time claims read off a SCHEMA rather than off a run.** Install writes some of
the seven execution properties and omits the rest, so what an OMITTED field does
is half the contract; §2 measures all seven, paired. `once` does not remove its
hook — it fired 3 of 3 — and `asyncRewake` does not imply `async`, since it
still blocks. **Two of seven descriptions false**, which is the same base rate a
version number produced. The reassuring one: omitting `shell` does not put a
shell back in the channel when `args` is present. And an `async: true` hook was
measured **killed with its start written and its end never**, which is §7's
non-delivery hazard one boolean away.

**Round 3b also turns an inference into a measurement**: N hooks in ONE settings
file for one event run **concurrently**, 15 ms apart. §7a argued from the
schema's array-of-matcher-groups shape that two hooks sharing an identity would
open one file at once. They do.

**Round 3c (2026-08-19), same build, and it is the first round to measure this
tool rather than Claude Code.** `vibe monitor hook` itself, killed at a
controlled delay: 26 kills against a live process, at 4 KB and at 64 MB,
produced **nothing on disk or a whole record and never a torn one** — paired
against a file torn on purpose, so the verdict was reportable. It is a bound
rather than a proof: the window exists and was never sampled inside. And the
dependent that predates install is now checked rather than reasoned — one
damaged file does **not** cost the sink its other records, with the reader's
four arms kept distinct and the control sabotaged red.

**The round also closes one omission-dependency and confirms another.**
`matcher: "*"` is a match-all and is accepted, so install writes it. `if: "*"`
is **accepted by the loader and fires nothing**, so `if` has no written value
meaning *do not suppress* and is the one residual — registered with its failure
direction, which is suppression.

**And it declares a limit the earlier rounds left implicit: every measurement of
Claude Code here is `win32-x64`, one binary, one machine.** Constraint 3 makes
three platforms first-class. The half with a dependent — what a damaged file
costs the rest of the sink — is Rust and runs on all three in CI. The half that
needs a live agent session cannot, and §9 says so rather than leaving the
register reading as if these were properties of the tool.

**Round 3e (2026-08-19) relabels three claims, moves two findings into CI, and
finds a defect in shipped code.** The 1 ms kill walk was **false precision** —
the delay is counted from spawn, and this round's own cold-start figures put the
spawn-to-work spread at ~26 ms against a 1 ms step, so the sweep re-rolled its
origin every run rather than walking a boundary. Relabelled, not redone. The two
structural findings — `write_all` never looping, and the observer resolving a
live partial state — are measurements of **this** code, so §9's single-platform
limit never applied to them and they are now controls on all three platforms.
`Stop` fires **per turn**, measured rather than read, which is what the
`timeout` cost argument rests on.

**And the same two-turn fixture found `SessionEnd` is not terminal**: a resumed
session emits `SessionStart` **after** `SessionEnd` under one `session_id`. That
is §4's *"the last event is never terminal"* arriving at retention, where
`Prunability` derives *prunable* from exactly that. Nothing renders it yet, so
it is recorded as a defect with a decision attached rather than repaired in
passing.

**Round 3f (2026-08-19) found the destructive window where nobody had been
looking: vibe's own `apply`.** `FileOp::UpdateFile` wrote through
`std::fs::write`, which truncates before writing, so the target was observably
**zero bytes** part way through — a window that exists **by construction**, not
a race. Three rounds had gone into whether a killed hook could tear a record in
a sink that appends and whose reader tolerates damage, while this sat in the
path that rewrites a file **vibe does not own** for a reader with no tolerance
at all. Repaired with a temp file beside the target and a rename over it,
measured with a paired control that catches the truncating path mid-replacement.
It repairs the manifest write too, which had the same window since ADR-0001.

**And round 3e's proposed repair was checked before being built, which killed
it.** *Ended at least once, reopenable* carries no information, because
`reopenable` is always true — a session two hours old resumed from a different
directory. So prunability is **not derivable from event content at all**, which
is a simpler and stronger finding than a third variant.

**Round 3g (2026-08-19) retracts prunability entirely and corrects a report
rather than a mechanism.** The third state was checked and carries no
information, so the type is deleted, the two controls that pinned it are
withdrawn with their subject, and **what must not be written in its place —
file-age prunability — is recorded where the next reader will stand**. Round 3f
also reported the sink's two-component filename as if it contradicted the
declared key: it does not, §7a encodes *no agent* by the component count, and
the separator is refused inside every component so the count decides. The report
was wrong and the code was right, which is worth recording in a document whose
subject is claims outliving what produced them.

**Round 3h (2026-08-19) finds the filename encoding is NOT INJECTIVE**, and the
guard §7a wrote to exclude the twin writer covers one form of the hazard.
`validate_component` refuses a literal `__` inside a component and permits a
single `_`, so two distinct triples can form one filename at a boundary —
constructed, with every component accepted. **`identity` is user-declared**,
which makes one half of every such collision a choice rather than a coincidence;
the other half needs a machine id ending in `_`, which the measured UUID and hex
ids cannot produce **today**. The repair is a product decision with a
user-visible refusal attached and is not taken here, with one shape refused in
advance: widening the check to `___` and `____` is filtering invalid states
rather than making them unrepresentable.

**Round 3d (2026-08-19) refines round 3c rather than extending it, and the
refinement changed three answers.** Reading the write path first turned out to
decide more than the sweep did: there is **no `BufWriter`**, and `write_all`
issues **one** `write` call at every size to 64 MiB, so the user-space window a
kill could tear a record in **does not exist** — a structural answer that
outranks any amount of sampling. The kill sweep's positive control was a
**static** truncated file where the target is a **live** one, so it controlled
the classifier and not the observer; a live half-write now shows the observer
resolving a 30-of-60-byte state, which is what makes the zeros about the
subject. And *"never sampled inside"* was an unfinished measurement rather than
a limit: fifteen further kills at 1 ms steps straddle the transition and none
tore. The 4 KB row is relabelled **unreached** — every kill landed before a byte
hit disk, so it was a guard never exercised, and the realistic size is covered
by the structural argument instead.

**One field measurement blocked install and is now cleared, on the right
class.** `matcher: "*"` had been measured on a **tool** event, where a matcher
filters tool names; install writes lifecycle events, where it filters something
else or nothing. Re-measured on all five inside install's own group: it fires,
1:1 against a no-matcher control. **`shell: false` is not expressible** — the
field is a string enum and the loader refuses it per hook — so `shell`'s default
is closed by writing `args`, not by writing `shell`, and §9's platform limit is
reworded from a live risk to a finding install does not depend on.

**And `File::flush` was measured to do nothing** — ~0 ns against `sync_all`'s
~90 µs — so `WriteStage::Flush` is a branch that **cannot be taken**. §9's
declared gap said it was hard to induce. Whether to delete the variant or keep
it as a declared dead branch is recorded as a decision rather than taken.

## Context

The feature is seeing which Claude Code instances are running, what they are
doing, and whether they have stopped. It shares a measurement round with
[ADR-0010](0010-per-project-prompts.md) and none of its decisions.

Versions, because a property belongs to a build:

| | |
| --- | --- |
| Claude Code, round 1 (2026-08-12) | **2.1.228**, native binary bundled in the VSCode extension |
| Claude Code, round 2 (2026-08-17) | **2.1.233**, commit `f8d57569aaf3`, same bundling |
| OS | Windows 10 Pro 19045; node v24.16.0 |
| Windows session | **non-elevated**, which §5's start-time fixture depends on |
| Codex | **not installed — nothing measured**, per ADR-0010 §1 |

**Every claim below carries its round.** Where round 2 contradicts round 1 the
contradiction is recorded as such rather than resolved by overwriting, because
which build a behaviour belongs to is the fact, and a document that silently
adopts the newest answer has thrown away the only thing a version-pinned
measurement was for.

## Decision

### 1. Agents report their own state; nothing is inferred from silence

Taken in principle before the measurement and confirmed by it. State is read
from **hooks**, never from process liveness or transcript mtime.

The argument is constraint 5. Thinking, running a long tool, waiting for
approval, having finished, and having crashed **all look identical from the
outside** — they are all silence. Inferring "stopped" from silence is inventing
a plausible value, and it is the same invention whether the source is a quiet
process or an unchanged file. Hooks replace it with reported facts.

### 2. Hooks fire on Windows, and both settings files load as a union

Measured by installing a node hook that appends its stdin verbatim, then
invoking the agent headlessly.

**Six event types observed in one run (round 1, 2.1.228):** `SessionStart`,
`UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SessionEnd`.

**`settings.json` and `settings.local.json` both load and both fire — a union,
not an override.** The two events declared in both produced **two entries each**,
distinguished only by a label the hook itself supplied.

That is a design constraint, not a curiosity: **an event can be reported more
than once, and the duplicate is not distinguishable by content.** Anything that
counts events, or that treats an event as a state transition, must deduplicate
on something the payload carries — `tool_use_id` for the tool pair, `prompt_id`
for the turn, `session_id` plus event name otherwise. A design that assumes one
delivery per event is wrong on any machine where both files declare hooks, which
is the ordinary case once a user has local overrides.

**Not observed, and recorded as unobserved rather than absent:** `Notification`
never fired, because no notification occurred — the positive control is that
seven other entries from the same config were written in the same run.
`PermissionRequest` was never exercised at all.

**Round 2 (2.1.233): nine event types, and the union re-established rather than
inherited.** *Added 2026-08-17.* Three runs against a fixture whose two settings
files both declare hooks — so the union is a **constructed** precondition, which
is what §9's dedupe obligation requires and what a fixture with hooks in one file
only would have failed to provide. Every event fired **twice**, once per file,
distinguishable only by a label the hook itself supplied. Round 1's finding holds
on this build.

Two types round 1 does not record: **`MessageDisplay`** and **`PostToolBatch`**.
So the observed set is nine and the earlier six was never a closed enumeration —
it was the list of what one run happened to produce, which is the ordinary rate
at which these tables turn out to be enumerations of what was tried (ADR-0010 §5).

**The build enumerates its own valid event names, and that is a different kind of
fact from an observation.** `claude doctor` reports an unknown hook event by name
and prints the valid set — **31 names on 2.1.233**, including `PermissionRequest`,
`PermissionDenied`, `Notification`, `SubagentStart`, `Elicitation` and
`TeammateIdle`. That enumeration says which names the build *accepts*; it says
nothing about which ones *fire*. The two are kept apart deliberately, because
collapsing them is how a schema becomes a behavioural claim.

**It does close one hole, and that is why it was worth running.** A negative
result about an event that never fired is worthless if the name was never
registered — the failure would be in this project's config and would arrive
labelled as a fact about the subject. Paired both ways: a deliberately bogus name
(`VibeNonexistentEventProbe`) is reported as *"Unknown hook event … was
ignored"*, and a file declaring only the real names produces no such report at
all. So §5's negatives below are about **firing**, not about a typo here.

**The hook *transport* is not one thing on this build, and §7 now depends on
that.** Five variants: `command` (with an `args` exec form that spawns directly,
**no shell in the channel** — used by every fixture in round 2), `prompt`,
`agent`, `http` (POSTs the hook JSON to a URL), and `mcp_tool`. Each carries
`timeout`, and `command` additionally carries `async` / `asyncRewake`.

**Round 3 (2.1.234): ten event types, and the version was measured rather than
assumed.** *Added 2026-08-19.*

**Which binary, established three ways before anything was counted.** Round 2's
number is recorded against 2.1.233 and that build is no longer on this machine,
so the first question was not *what fires* but *what is running*. Two versions
are installed — `2.1.234` and `2.1.235`, the latter placed on disk the morning of
the measurement — and **the session was running the older one**, so *"current"*
and *"invoked"* are different answers here and only one of them is a fact about
the run. Established by three independent instruments that agree:
walking the parent chain of the measuring shell through `Win32_Process` reaches
`.vscode/extensions/anthropic.claude-code-2.1.234-win32-x64/resources/native-binary/claude.exe`;
the inherited `CLAUDE_CODE_EXECPATH` names the same file; and that file's own
`--version` reports `2.1.234 (Claude Code)`, with `claude doctor` adding commit
`7215ba60b06d`. **Recorded as a version plus a path**, because a version alone
would not have distinguished the two builds sitting side by side.

**Ten types fired**, against a fixture whose two settings files each declare a
`command` hook in the `args` exec form for **all 31** accepted names:
`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolBatch`,
`MessageDisplay`, `SubagentStart`, `SubagentStop`, `Stop`, `SessionEnd`.

**The union re-established a third time, and constructed rather than inherited.**
Every one of the ten fired **exactly twice**, once per settings file,
distinguishable only by the label the hook itself supplied — 38 invocations for
19 events in the first run, and the same 2:1 ratio in every later run.
`claude doctor` also reports a config fault **once per file**, which is the union
visible at config-read time as well as at fire time.

**Ten is not nine plus one, and the difference is what the fixture did.**
Round 2's nine and this ten are both enumerations of what one fixture produced.
This fixture spawned a subagent, so `SubagentStart` and `SubagentStop` fired;
nothing here says they did not fire on 2.1.233, and nothing says the twenty-one
names that stayed silent cannot fire.

**The label for those twenty-one is *neither fired nor shown to be correctly
registered*, and the two are not distinguished.** *Corrected 2026-08-19; the
first draft said "unconstructed, not absent", which claims one step past the
instrument.* There are three possibilities and this round separates none of
them: the name has no dispatch site on this build, or it has one this fixture
never triggered, or **this fixture declared it in a shape that cannot fire** —
every group here omits `matcher`, and nothing establishes that every event is
reachable without one.

**What *is* established, and it is narrower than registration:** `claude doctor`
reported no unknown-hook-event for any of the 31, so the settings loader
**accepted** every name. That rules out a typo. It does not rule out a wrong
declaration shape, and it says nothing about dispatch. The positive control for
the run as a whole is that ten other names from the same two files were written
in it.

**The name-registration control re-established on this build, paired both ways.**
A fixture declaring only real names produces no config fault at all; the same
fixture plus `VibeNonexistentEventProbe` is reported as *"Unknown hook event
… was ignored"* — by name, in **both** settings files — followed by the valid
set. So the twenty-one zeros above are about **firing**, not about a typo here.

**And the accepted-name enumeration now has two independent sources that
agree.** Besides `claude doctor`, the installed extension ships
`claude-code-settings.schema.json`, whose `hooks` node enumerates the same **31**
names. They are identical to each other, and **identical between 2.1.234 and
2.1.235** — so the schema did not move across the version boundary this
measurement straddles. It remains a statement about which names the build
*accepts*.

**The `command` variant carries ten properties, not the three §7a names.**
Measured from the same schema and identical on both builds:
`type`, `command`, `args`, `if`, `shell`, `timeout`, `statusMessage`, `once`,
`async`, `asyncRewake` — with `matcher` one level up, on the group. §7a's
contract bullet names `timeout`, `async` and `asyncRewake`; **`once` and the
`if`/`matcher` pair belong to the same class and are not named there.** `once`
removes the hook after a single execution, and `if`/`matcher` suppress the
invocation entirely. Both decide *whether* a record arrives, which is §7a's own
test for what the contract must pin, and both make an absence of records mean
something other than an absence of events.

**No event type carries a timestamp on this build either.** Checked across every
field of all ten types: nothing naming a point in time. §7a's ordering argument
rests on that measurement, and it is re-established here on ten types rather than
inherited from eight.

**Round 3b (2.1.234): what a hook does when a field is OMITTED, which is the
half the schema does not answer.** *Added 2026-08-19.* Install writes some of
the seven execution properties and omits the rest, and an omitted field is not
*off* — it is whatever this build defaults to. The descriptions above were read
off the schema; these are measured, each paired against the same field written
explicitly, in one session over one settings file so the two observables come
from the same run.

**The probe emits a line before its dwell and a line after**, so a hook that is
killed is a **start without an end** rather than an absence — and an absence is
what a hook that never fired also produces. The two must not share an
observable, which is the same rule §5's NULL-handle bullet applies to
`OpenProcess`.

| field | omitted | written | verdict |
| --- | --- | --- | --- |
| `timeout` | a 65 s hook **completed** — no kill observed at 65 s | `2` kills a 5 s hook (start, no end); `30` and `120` let 5 s and 65 s through | kill confirmed; **the default bound is not located** |
| `async` | **blocking** — session wall 26.8 s against a 6.8 s baseline | `true` → session returns at 6.8 s, and the hook is **killed with its start written and its end never** | holds, and the loss is real and permanent |
| `asyncRewake` | blocking | `true` → **still blocking**, 26.8 s | **the schema's *"Implies async"* is FALSE on this build** |
| `once` | fires every time | `true` → **fired 3 of 3 tool calls**, three distinct pids, one session | **the schema's *"runs once and is removed"* is FALSE on this build** |
| `if` | fires for everything | matching → 3; non-matching → **0** | holds |
| `matcher` | fires for everything | matching → 3; non-matching → **0** | holds |
| `shell` | with `args`: **no shell**; without `args`: **bash** | with `args`: `bash` and `powershell` both produce the **identical literal argv** | holds for the string form; **inert when `args` is present** |

**Two of seven descriptions are false, which is the base rate arriving on
schedule.** The nine was documentation-of-a-run and it was withdrawn; `once` and
`asyncRewake` were documentation-of-a-property and they are withdrawn here. A
schema description is a claim about a build like any other.

**The `shell` result is the one install depends on and it is the reassuring
one.** §7a chose the `args` exec form to keep a shell out of the channel, and
omitting `shell` does **not** put one back: an argument written as
`A$(whoami)B` + backtick + `C` + backtick + `D&E|F` arrives byte-identical under
`shell` omitted, `shell: "bash"` and `shell: "powershell"` alike. Two
instruments agree — the argv as received, and `SHLVL`, which stays at the
launcher's value for the exec form and is incremented for the string form.
Measured paired: the **string** form (no `args`) does interpose a shell, a `;`
chain runs both halves, and a PowerShell call-operator form runs nothing, so it
is bash on this machine.

**`once` does not rewrite the settings file.** The file's hash is byte-identical
before and after a session in which a `once: true` hook fired three times. Worth
recording because the alternative — a tool editing a file vibe also edits —
would have been an idempotency problem install could not see.

**Two things the table above must not be read as saying.** *Added 2026-08-19.*

**`timeout`'s default bound is NOT located.** A 65 s hook was not killed. That
is where the observation stops: nothing here finds the bound, and *"the default
is permissive"* would be the same shape as reading a version number off the
newest install rather than off the running one. The honest label is **no kill
observed at 65 s; the default bound is not located**, and install writes an
explicit `timeout` rather than depending on it (§7b).

**`shell`'s inertness has a precondition, and the precondition is `args`.**
`shell` written and `shell` omitted produce identical argv **only while `args`
is present**. Recorded as a dependency rather than as a property: a later change
that drops `args` — for a `command` string that is easier to read, say — makes
`shell` live again, puts a shell back in the channel, and **nothing would turn
red**. The exec form is load-bearing, not stylistic.

**And the two instruments do not have equal reach, which *"two instruments
agree"* concealed.** `SHLVL` is a bash variable: it is evidence when bash is the
interposed shell and says **nothing at all** under PowerShell, where its absence
is indistinguishable from no shell. So the settings are covered unevenly, and it
is stated per setting rather than in aggregate:

| setting | argv literal | SHLVL |
| --- | --- | --- |
| `args` present, `shell` omitted | covered | covered (bash absent, and bash is what the default selects here) |
| `args` present, `shell: "bash"` | covered | covered |
| `args` present, `shell: "powershell"` | **covered, and alone** | no reach |
| no `args` (string form) | covered | covered |

The argv-literal instrument carries every row. `SHLVL` corroborates three of
four and is blind on the one where a *different* shell would be the finding.

**Round 3c (2.1.234): what a kill leaves on disk, measured against the real
hook.** *Added 2026-08-19.* §7a admits exactly one corruption under
one-writer-per-file — truncation — and the round above measured that a `timeout`
really does kill. What decides whether a short `timeout` is admissible is
whether a kill can land **inside** the append.

The subject is `vibe monitor hook` itself, killed at a controlled delay, with
the file classified afterwards into three outcomes that do not share an
observable: **nothing on disk**, **a whole record**, or **a torn one**.

| payload | kills that landed on a live process | nothing | whole | torn |
| --- | --- | --- | --- | --- |
| 4 KB | 6 | 6 | 0 | **0** |
| 64 MB | 20 | 10 | 10 | **0** |

The 64 MB sweep walks the transition in 15 ms steps: at 230 ms nothing is on
disk, at 245 ms a complete 67,109,142-byte record is. **No delay produced a
partial file at any size tried**, so a 64 MB `write_all` is narrower than the
sampling interval.

**Paired, or the zero is a skipped test.** A file torn deliberately —
half a record, no trailing newline — is classified **TORN** by the same code
path. The instrument can report the verdict it never reported.

**What that licenses and what it does not.** On **Windows 10 Pro 19045, NTFS**,
26 kills against a live hook produced no torn record. It is **not** a proof that
a kill cannot tear one: the window exists, it was never sampled inside, and
nothing here was run on Linux or macOS. It is a bound — *the window is smaller
than 15 ms for a 64 MB write on this platform* — and a bound is what a `timeout`
value can be chosen against.

**And the dependent that predates install is now checked rather than reasoned.**
The worry was that one torn line might darken the whole sink, which would make
any kill a permanent blackout. It does not, and the reader's arms are distinct:
`read_sink` returns an error **only** when the directory cannot be enumerated; a
file that cannot be *read* is reported as `Unreadable` and skipped alone; a torn
trailing line is `TailState::Partial` with every whole record before it kept; a
whole line that does not parse is counted as `unparseable` with its neighbours
kept. Four files in one sink, three damaged differently, every whole record
still read — `one_damaged_file_does_not_cost_the_sink_its_other_records`, paired
against an undamaged sink, and sabotaged red by making the reader skip a torn
file.

**A match-all `matcher` exists; a match-all `if` was not found.** Measured on a
session using two tools, against a no-matcher control that fired twice:

| declaration | fired |
| --- | --- |
| nothing (control) | 2 of 2 |
| `matcher: "*"` | **2 of 2** |
| `matcher: ""` | 2 of 2 |
| `matcher: ".*"` | 2 of 2 |
| `if: "*"` | **0 of 2** |
| `if: "Read(*)"` | 1 of 2 |

So `matcher` has a written value meaning *do not suppress* and `if` does not —
`"*"` is **accepted by the loader and suppresses everything**, which is the
worst of the three possible answers: not a syntax error anyone would see, just
silence. `if` is therefore the one residual omission-dependency (§7b), and its
failure direction is suppression.

**Round 3d (2026-08-19): the write path read before the measurement was refined,
and it decides more than the sweep did.** *Added 2026-08-19. Three of round 3c's
statements were weaker than they read, and this is the repair.*

**The syscall shape, from the code and then measured.** `Writer::append` opens
with `OpenOptions::new().append(true).create(true)`, which yields a bare
`std::fs::File`. **There is no `BufWriter` anywhere in the path.** It then calls
`write_all` once and `flush` once. `write_all` loops over `Write::write` until
the buffer is consumed, so the question that decides whether a record can be
torn **in user space** is whether one `write` takes the whole buffer.

Measured on `windows/x86_64`, one call each:

| bytes asked | accepted by ONE `write` | `write_all` calls |
| --- | --- | --- |
| 327 (a real record) | 327 | **1** |
| 4 KiB | 4 KiB | **1** |
| 64 KiB | 64 KiB | **1** |
| 1 MiB | 1 MiB | **1** |
| 16 MiB | 16 MiB | **1** |
| 64 MiB | 64 MiB | **1** |

**So `write_all` never looped, and the user-space window a kill could land in
does not exist at any size tried.** That is a structural answer and it outranks
the sampling: making the state unrepresentable beats filtering for it, which is
the same move as the missing `FileOp::Delete`. What it does **not** settle is
whether the kernel can leave a single `WriteFile` partially applied when the
process is terminated — that is the residual, and the sweep below is what
addresses it.

**`File::flush` does nothing, and that makes `WriteStage::Flush` unreachable
rather than untested.** Measured against a call known to issue a syscall:
`flush` costs **~0 ns** per call over 100,000 calls; `sync_all` costs **~90 µs**.
With no user-space buffer in the path there is nothing for it to flush. §9's
declared gap records `Append` and `Flush` together as *"cannot be induced from
this machine"*; for `Flush` that reason is wrong — it is not hard to induce, it
**cannot occur**. Whether the right repair is to delete the variant, making the
unreachable state unrepresentable as this document does elsewhere, or to keep it
as a declared dead branch, is a decision rather than a detail.

**The positive control for the kill sweep was STATIC and the target is LIVE.**
*This is the correction that mattered.* Round 3c's control was a file truncated
on purpose — held open by nobody. The target is a file a live process is
mid-write on, and on NTFS an observer may see a cached view, be denied, or see
the size update only at completion. The control proved the **classifier**
recognises a torn file; it proved nothing about whether the **observer** can see
one being made, and a blind observer produces the same clean sweep a healthy one
does.

**Measured with a known live input, and the observer is not blind.** A separate
process appends 30 of a 60-byte record, holds the handle open, waits, then
writes the rest. During the hold the observer reads **30 bytes, not whole**;
after the writer exits it reads **60 bytes, whole**. Paired in both directions,
so *"sees half"* is not satisfied by an observer that reports half
unconditionally. The sweep's zeros are therefore about the subject.

**The window is now finished rather than declared.** Round 3c filed *"never
sampled inside"* as a limit; it was an unfinished measurement, and the untested
part was 15 ms wide. Fifteen further kills at **1 ms** steps across 231–245 ms
straddle the transition — the boundary jitters, with complete records at 232,
238, 240, 241 and 243–245 and nothing on disk between them — and **not one
produced a partial file**.

| fixture | kills landing on a live process | nothing | whole | torn |
| --- | --- | --- | --- | --- |
| 4 KB, 5 ms | 6 | 6 | 0 | **unreached** |
| 64 MB, 180–700 ms coarse | 20 | 10 | 10 | 0 |
| 64 MB, 231–245 ms at 1 ms | 15 | 8 | 7 | 0 |

**The 4 KB row is relabelled: it is unreached, not a zero.** All six kills landed
before any byte hit disk, so the fixture never reached the window and a zero
there is a guard never exercised. 4 KB is near the real record size, so **the
realistic size is not measured for tearing at all** — it is covered instead by
the structural argument above, which is the stronger of the two and does not
depend on hitting a window.

**A match-all `matcher` holds on the lifecycle five, measured on the class
install writes.** Round 3c measured `matcher: "*"` on a **tool** event, where a
matcher filters tool names; on `SessionStart` it filters something else or
nothing. Installing against the first while shipping the second is a control
proving one hazard class and shipping against another. Re-measured inside the
group install writes, one session that spawns a subagent, paired against a
no-matcher group in the same file:

| event | no matcher | `matcher: "*"` |
| --- | --- | --- |
| `SessionStart` | 1 | **1** |
| `SessionEnd` | 1 | **1** |
| `SubagentStart` | 1 | **1** |
| `SubagentStop` | 1 | **1** |
| `Stop` | 1 | **1** |

The loader also accepts a matcher on all five with no complaint — recorded
separately, because silent acceptance and silent non-match are the same
observable from outside and only the firing table separates them.

**`shell` has no value meaning *no shell*, and `shell: false` is refused.**
Measured from the schema — `shell` is a string enum of `"bash"` and
`"powershell"` — and from the loader, which reports *"Invalid value. Expected
one of: bash, powershell"* per hook. **So `shell`'s default is closed by writing
`args`, not by writing `shell`**, and the dependency install carries is on the
exec form. Unlike `if: "*"`, this failure is loud.

**And the whole hook install will write was run end to end.** One group carrying
`matcher: "*"`, `once: false`, `async: false`, `asyncRewake: false`,
`timeout: 5` in the `args` exec form: **accepted by the loader with no
complaint, and fired on all five events, 1:1 against a bare control in the same
file.** `asyncRewake: false` is accepted alongside `async: false`.

**Round 3e (2026-08-19): three relabels, one defect found in shipped code, and
the structural findings moved into CI.** *Added 2026-08-19.*

**The 1 ms walk was false precision, and the measurement proving it is in the
same round.** The kill delay is counted from **spawn**, not from the start of
the write, and this round's cold-start figures put the spread between spawn and
work at **11.7–37.5 ms** — roughly 26 ms of jitter against a 1 ms step. So the
sweep was not walking a boundary at fine resolution; it was **re-rolling the
origin every run under noise 26 times the step size**. That is why the boundary
appeared to tremble — whole at 232, nothing through 237, whole at 238. The
origin was moving, not the boundary. Same class as the synthetic overlap control
decided by interpreter startup.

**Relabelled rather than redone**, because randomized coverage is still
coverage and the structural argument carries the weight now: **35 kills at
64 MB, origin randomized by cold-start jitter of about 26 ms, effective
sampling random rather than a 1 ms walk, zero partial observed.**

**The structural findings are now controls, on three platforms.** Both are
measurements of **this** code rather than of a live agent session, so §9's
single-platform limit never applied to them and they mechanize:

- `one_write_call_takes_a_whole_record_on_this_platform` — one `write` must take
  the whole buffer at every size to 4 MiB, or `write_all` loops and the
  user-space window opens.
- `an_observer_can_see_a_partial_write_on_a_live_file` — the observer control,
  cross-process, with a **file handshake rather than a sleep** so its firing does
  not depend on winning a race. It re-invokes the test binary rather than adding
  a helper to the shipped executable, because a flag that exists only for a test
  is a thing users can find.

If some platform's `write` returns short, that is the finding and it arrives as
a red rather than as a surprise in a record.

**`Stop` fires per turn — measured, because the `timeout` cost argument rests on
it.** Two prompts in one session, the second by `--resume`: **`Stop` fired
twice**, and so did `UserPromptSubmit`. The claim was previously read off a
schema, which is the standing this round retired twice already.

**And the same fixture found a defect in shipped code.** `SessionStart` and
`SessionEnd` **also** fired twice, under **one** `session_id`, in this order:

```
+0 ms      SessionStart
+973 ms    UserPromptSubmit
+2617 ms   Stop
+2738 ms   SessionEnd
+7915 ms   SessionStart      <- same session_id
+8083 ms   UserPromptSubmit
+10098 ms  Stop
+10208 ms  SessionEnd
```

**So `SessionEnd` is not terminal for a `session_id`, and events follow it.**
That is §4's *"the last event is never terminal"* arriving one level down, at
retention — and `Prunability` derives `Prunable` from *"this file contains
`SessionEnd`"*. **A resumed session's file would be offered as prunable while
the session is still live.** Nothing renders it today (§8 leaves the display
open), so the cost is bounded, and **what the label claims is a decision rather
than a repair to take here.** The shape of the repair is the one this document
uses everywhere: a third state, *ended at least once, and a resume can reopen
it*, never borrowing the appearance of *finished*.

**Cold start is bimodal, which the `timeout` rule has to know.** The first
invocation after a build measured **1.27 s** against a steady state of about
**22 ms** — the operating system loading a 5.6 MB binary written seconds
earlier. A rule that multiplies *the maximum* therefore multiplies whichever
mode the run sampled. The controls take **one untimed warm-up** and time the
steady state, printing the discarded number rather than hiding it; whether the
rule should name the cold mode instead is recorded in §7b as open.

**Round 3f (2026-08-19): the destructive window was in vibe's own `apply`, and
three rounds of tearing work had been aimed at the wrong file.** *Added
2026-08-19.*

**`FileOp::UpdateFile` wrote through `std::fs::write`**, which is `File::create`
plus `write_all` — and `File::create` **truncates before any byte is written**.
So there was a window where the target is **zero bytes**, and unlike the kernel
window this document sampled 35 times without finding, **this one exists by
construction**: not a race that might not happen, a state the sequence passes
through every time.

**The asymmetry is the finding.** Rounds 3c to 3e went into whether a killed
hook could tear a record in vibe's **own sink**, where the writer appends, the
reader tolerates damage, and every whole record before the damage survives.
None of that transfers to `apply`: it rewrites files whose readers have no
tolerance at all — `.claude/settings.json` is read by a strict JSON loader — and
one of them is a file **vibe does not own**. Hard constraint 2 is not *"there is
no `FileOp::Delete`"*; the absent variant is how the constraint is **enforced**,
and the constraint is **never destructive**. A zero-byte `settings.json` is
destructive on any reading.

**Repaired by writing to a temporary file beside the target and renaming over
it**, and **measured rather than read off documentation** — *"rename is atomic"*
is exactly the class of cross-platform claim that died on contact with
measurement in ADR-0002 §7. A reader spins on the target through 400
replacements and reports every distinct state it sees:

| write mode | states observed |
| --- | --- |
| `std::fs::write` | `Empty`, `WholeOld`, `WholeNew` |
| temp + rename | `WholeOld`, `WholeNew` |

**The negative half is what licenses the positive one.** A reader too slow to
catch anything reports a clean sweep too, so the identical reader runs against
the truncating path and **must** catch it mid-replacement. It does — `Empty`.
Both run in the ordinary test job, so this is carried on all three platforms.

**Beside the target, not in the system temp directory**, and that is
load-bearing: a rename across volumes is a copy plus a delete, which puts the
window back and adds a delete to a tool that has none. Asserted by watching the
directory during the write rather than by reading the implementation. **What it
does not promise is durability** — there is no `fsync`, so a power failure can
still lose the new contents; what it cannot do is leave the target empty or half
written.

**This also repairs the manifest path**, which had the same window and has had
it since ADR-0001. Recorded because the editor is what made anyone look.

**PRUNABILITY IS NOT DERIVABLE FROM EVENT CONTENT AT ALL, and the third state
would have carried no information.** Round 3e found `SessionEnd` is not terminal
and proposed *ended-at-least-once, reopenable* as the repair. Checked before
building it, which is the right order: **`reopenable` is always true.** A
session that had already emitted `SessionEnd` was resumed minutes later; a
different session **two hours old** was resumed **from a different working
directory**, with its `session_id` preserved and no error. Nothing in the
payload bounds it — the one `SessionEnd.reason` observed is `"other"`, and the
value set is unenumerated.

So a variant whose predicate is always true distinguishes nothing, and the
honest finding is the simpler one: **whether a file will receive more records is
not a function of the events in it.** Prunability has to come from somewhere
else — file age, or an explicit user action — or not be offered.

**The cost bound is structural, not situational, and the reason for the repair
is constraint 5.** *Corrected: round 3e said "nothing renders it yet", which
makes the bound depend on §8 staying open.* The real bound is that **no
`FileOp::Delete` exists**, so nothing vibe can do can delete a record whatever
the label says. The repair is owed anyway, because the label **claims what it
does not know** — and that survives §8 shipping, where *"nothing renders it"*
would not.

**The editor's own output was run end to end, into an existing file.** §2's
round 3d validated a **hand-written** group; the editor generates one, and those
are different artifacts. Against a 4-space `settings.json` already carrying a
user's own `PreToolUse` hook: the editor installed, **preserved the 4-space
indent**, left the user's hook intact, and the loader accepted it with no
complaint. A live session then delivered **all five lifecycle events** through
the real `vibe monitor hook` binary into the sink — and the three-part filename
key separated them for the first time against a real agent, producing
`<session>__user.jsonl` beside `<session>__<agent_id>__user.jsonl`.

**The cold-start number is kept rather than discarded.** *Corrected.* Calling it
a warm-up attributed it to the build; the cause is a **cold page cache**, which
recurs after a reboot or after the binary has sat unused — and `SessionStart` is
exactly where that lands. It is also the sample CI is best at: **every job builds
and then runs, so the first invocation in CI is always the cold one**, one clean
sample per push per platform. It is excluded from the steady-state population,
**asserted against a loose tripwire, and printed**, so the distribution
accumulates. Reproduced at **1.20 s** and **1.27 s** on `win32-x64`, debug, a
5.6 MB binary; release differs in size and has not been measured.

**And two counts of "how many tests" disagreed, so both were measured.**
`cargo test --workspace` prints one `test result:` line per **binary** — 25
integration targets plus 2 lib-unittest binaries plus 1 doc-test binary — while
`control_inventory.rs` counts **integration-test targets excluding itself**.
Neither was wrong; the report that put 28 beside 24 was. Measure the tool with
the tool.

**Round 3g (2026-08-19): the filename key, and a report that mis-read its own
result.** *Added 2026-08-19.*

**Round 3f reported two components as a success and did not notice it looked
like a contradiction.** The sink showed `<session>__user.jsonl` beside
`<session>__<agent_id>__user.jsonl`, and the report described the declared key as
`<session>__<agent>__<identity>`. **The report was wrong, not the code**: §7a
records that `agent_id` is absent on parent-level events and that its absence is
*"encoded structurally rather than by a reserved word"* — **two components is a
session-level record, three is an agent-level one**, and the count carries the
distinction. `Attribution::of` matches both arities. So the observation was the
design working, and the summary of it was not.

**The count only decides if no component can contain the separator, and that is
enforced rather than assumed.** **~~`validate_component` rejects any byte outside
`[A-Za-z0-9-_]` and rejects any component containing `__` outright
(`ComponentRejection::ContainsSeparator`). A single `_` is legal; two adjacent
are not.~~** *Retracted 2026-08-19 — that rule was the defect, see round 3h below:
a single `_` at a boundary let two legal components form the separator between
them.* The charset is `[A-Za-z0-9-]`, `_` is refused outright, and
`ComponentRejection::ContainsSeparator` was deleted for being unreachable. So two
distinct keys cannot collapse onto one filename — the twin writer §7a exists to
exclude — and it is now a construction rather than a property of the values that
happen to arrive, which is what the earlier version only appeared to be.

**Measured on the values that do arrive**, since the enforcement is only
interesting if real ids pass it: across five distinct real `session_id`s and five
real `agent_id`s captured from live sessions —

| field | n | max length | observed charset | contains `__` |
| --- | --- | --- | --- | --- |
| `session_id` | 5 | 36 | `[0-9a-f-]` (a UUID) | 0 |
| `agent_id` | 5 | 17 | `[0-9a-f]` | 0 |

Both inside the accepted set and inside the length bounds (64 and 48), so the
charset check refuses nothing real today. **That is a sample, not a guarantee
about the fields**, which is why the check exists: a value outside the set is
refused rather than assumed impossible.

**And the rename's argument is structural, not sampled — which round 3f left
implicit.** The spinning-reader result is bounded by reader resolution: the
truncating window is a `File::create` plus a ~500-byte `write_all` and is long;
a rename's is orders of magnitude shorter, and a reader shown to sample inside a
long window is **not** shown to sample inside a short one. What carries the claim
is the specification: on POSIX `rename(2)` is atomic; on Windows
`std::fs::rename` calls **`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`**.

**Measured on Windows 10 Pro 19045, because the Windows half is where the answer
is least obvious — and it fails, safely, in one case:**

| the holder's share mode | rename-over | `DeleteFile` |
| --- | --- | --- |
| `FileShare.None` | **refused**, original intact | refused |
| `FileShare.Read` | **refused**, original intact | refused |
| `FileShare.ReadWrite, Delete` (Rust's `File::open`) | succeeded | succeeded |

**Two things follow.** The failure direction is the safe one — an error, with the
user's file untouched. And `cache.rs`'s long-standing fallback, which **deleted
the destination and retried** under the comment *"Windows will not rename onto an
existing file in every case"*, was **inert where it was aimed**: `DeleteFile` is
refused in exactly the two cases the rename is, so the retry could never have
helped, and if it had fired it would have left the destination *missing*.

**And the case install actually meets was measured rather than feared:** install
runs while Claude Code runs, so `claude doctor` was run against a settings file
while replacements ran in a loop. **35 replacements, 0 refused.** Claude Code
does not hold `settings.json` in a way that blocks the write.

**Round 3h (2026-08-19): the filename encoding is not injective, and the guard
that was supposed to prevent that covers one form of the hazard.** *Added
2026-08-19.*

**Constructed, not reasoned.** Two distinct triples, every component accepted by
`validate_component`, producing one filename:

```
("sess", agent "abc_",  identity "user")   ->  sess__abc___user.jsonl
("sess", agent "abc",   identity "_user")  ->  sess__abc___user.jsonl
```

**That is the twin writer §7a exists to make unrepresentable, arriving through
the check that was written to exclude it.** The check refuses a component
containing a **literal `__`** and permits a single `_`, so it covers *separator
inside a component* and not *separator formed at a boundary*. The two-component
form has the same defect and adds a misattribution: session `sess_` with identity
`X`, and session `sess` with identity `_X`, both render `sess___X.jsonl`, which
`Attribution::of` parses as `("sess", "_X")` — so one of the two writers is read
back under a session it does not belong to.

**Where `identity` comes from, since that decides whether this is
input-reachable.** It is **user-declared**. It arrives as `--identity` in the
hook's argv (§7a: *"the identity the hook itself declares, one per installed
hook"*), written either by `vibe monitor install` or **by hand** — §7 permits
hand-installed hooks explicitly, and §7a's uniqueness check exists because more
than one identity can exist. So **one half of every collision above is chosen by
a person**, not stumbled into.

**The other half is not reachable from the ids measured today, and that is a
property of the sample.** `session_id` is a UUID and `agent_id` is 17 hex
characters (§2, round 3g), so neither can end in `_`. **This is exactly the
standing this document refuses elsewhere**: the charset accepts `_` in those
components, nothing upstream promises hex, and *"the values we have seen cannot
trigger it"* is what round 3g's own sample note says not to lean on.

**Two repair shapes, and one of them is refused now so it does not get proposed
later.**

- **Refused: widen the refusal to `___`, `____`, and so on.** That is filtering
  invalid states rather than making them unrepresentable, and the enumeration has
  no end — the same move ADR-0005 §10 rule 4 rejects for URLs, for the same
  reason.
- **Forbid `_` inside every component**, leaving `[A-Za-z0-9-]`. Then `__` cannot
  be **formed** from component content at all, at a boundary or inside, and the
  concatenation is injective by construction. It is the closed-allowlist move the
  charset already is, applied one notch tighter. **The cost is real and small**:
  an identity like `my_hook` is refused and has to be `my-hook`, which is a
  message at install rather than a silent loss — and it is a message a
  hand-installer sees too, because the same validation runs at write.
- **Or make the encoding unambiguous some other way** — a length prefix, or an
  escape — at the cost of a filename that stops being readable, which §7a values.

**DECIDED 2026-08-19: forbid `_`.** Charset `[A-Za-z0-9-]`, injectivity taken,
rename paid. **The deciding reason is standing, not cost** — the alternative's
safety rests on *measured ids are UUID and hex, n=5*, and an upstream change to
the agent-id format must not be able to reopen a file collision. `_` removal
needs no knowledge of what id formats look like today.

**It is a breaking change whose cost is zero for a reason that EXPIRES.**
`[A-Za-z0-9-]` refuses hand-written identities containing `_` and makes any
filename already written with one unparseable. That costs nothing **because the
monitor is unshipped** — not because migration is handled — and the reason stops
holding the moment §8 ships a display and somebody has a sink on disk. Recorded
here rather than assumed, because a zero-cost breaking change is the kind that
gets repeated once it stops being zero-cost.

**The degradation is old; the POPULATION is new.** *Added 2026-08-19.* Before
this change a filename with `_` in a component was **not producible by vibe** —
it could only come from a hand-written hook, which §7 admits and which is a small
and self-selected group. It is now producible by **any install predating the
change**. Not a new gap: the same gap with a larger population, and after §8 that
population includes real users. Recorded beside the expiry above because the two
age together — the day the zero-cost reason stops holding is the day the
population stops being empty.

**And `read_sink` does meet legacy-form files, so that case is not hypothetical.**
`Attribution::of` parses every component, so a name containing `_` now fails and
the file reads as **`unattributed`** — which is the degradation §7a already
designed for rather than a new one: the events are real, the session is named in
every payload, only the *source* is unknown, and **every record inside is still
read**. Pinned by
`a_filename_from_the_old_charset_is_unattributed_and_keeps_its_records`, paired
against the same records under a name that parses.

**Three things it carries**, recorded so none of them survives as a separate
gap: the **two-component misattribution** dies with the same change, since
`sess___X` can no longer be formed; the refusal is **loud at install and silent
in a hand-installed hook**, so the risk moves rather than disappears; and **the
tool does not normalise** — `my_hook` becoming `my-hook` is the person's edit,
because substituting silently is inventing a plausible value.
`ComponentRejection::ContainsSeparator` became unreachable and was deleted.

**And `validate_component`'s failure direction is recorded, because today it
reads as a guard and it is also a producer of silence.** A `session_id`,
`agent_id` or identity outside the charset is **refused, and the event is lost**.
§7a already says this for the `session_id` case and frames it as *"loud loss
beats silent invention"* — loud **to the hook's stderr**, which nobody reads, and
silent to vibe. So it belongs in the same list as the other non-delivery
producers: **a charset that is too narrow does not corrupt anything, it deletes
events.** n=5 on one build bounds nothing about the value space, and tightening
the charset — which the repair above would do — moves this risk rather than
removing it.

**Separately: nothing holds `settings.json` open during a live session.** Round
3g reported 35 replacements during a `claude doctor` run with 0 refused, and that
had **no reachability premise** — both are short, and if the read never
overlapped a replacement the zero was a guard never reached. The question
underneath is better asked directly, and it was: an exclusive open
(`FileShare.None`) was attempted every 20 ms throughout a live headless session —
start, tool use, stop, end.

| run | attempts | exclusive open succeeded | refused |
| --- | --- | --- | --- |
| live session | 307 | **307** | 0 |
| **paired control** — the file held for 3 s on purpose | 89 | 1 | **88** |

The control is what makes the zero reportable: the instrument **can** see a
hold, and saw none.

**Recorded on its true footing, which is narrower than the first draft claimed.**
*Corrected 2026-08-19.* That draft said the sub-20 ms window *"is the
open-and-close case, which the replacement measurement already covers"* — and
the replacement measurement is the one this round declared to have **no
reachability premise**. The two were propping each other up, and neither covers
the sub-20 ms window. So:

> **No hold longer than 20 ms was observed in a live session, n=307, paired
> against a deliberate hold that the same instrument caught 88 times of 89. The
> sub-20 ms window is unsampled, and the replacement measurement does not cover
> it, because its own overlap premise is not established.**

**The practical conclusion survives on a different footing, and it is the better
one: argument, not sampling.** A rename refused because a holder forbids
delete-sharing **leaves the original intact**, reports an error, and now leaves
no temp behind. So the cost of the unsampled case is a **loud, non-destructive
failure of `vibe monitor install`**, which the user can retry — not a damaged
`settings.json`. That claim needs no measurement of how often it happens.

**And a fact that was inferred from the schema and is now measured: N hooks
declared in ONE settings file for one event run CONCURRENTLY.** Three hooks with
a 4 s dwell started within **15 ms** of each other and all ended together. §7a
already argues that one settings file can declare N×M command hooks and that
*"all of them would open one file, and concurrent append returns inside the
design whose entire justification is that it cannot occur"* — that was read off
the schema's array-of-matcher-groups shape. It is now a measurement: **two hooks
sharing a declared identity in one file are two writers to one file, at the same
time.** The uniqueness check is not defensive.

### 3. Attribution comes from the payload, never from the environment

Every payload carries `session_id`, `transcript_path` (absolute), `cwd`
(absolute), and `hook_event_name`; from `UserPromptSubmit` onward also
`prompt_id` and `permission_mode`. `PreToolUse` adds `tool_name`, the complete
`tool_input`, and `tool_use_id`; `PostToolUse` adds `tool_response` and
`duration_ms`; `Stop` adds `last_assistant_message`, `background_tasks`,
`session_crons` and `stop_hook_active`; `SessionEnd` adds `reason`.

**So attribution to a registered project needs no guessing and no environment
read**: `cwd` is the project directory, and `session_id` plus `transcript_path`
identify the run. **`PreToolUse` does say what the agent is doing** — the tool
name plus its full input, correlatable to its `PostToolUse` by `tool_use_id`.

**The environment is deliberately not used, and that is what dissolves a
caveat.** These payloads were captured from a fixture **nested inside a Claude
Code session**, and the child's environment was measured to be contaminated by
the parent's: `CLAUDE_CODE_EXECPATH` named 2.1.227, the version the outer session
ran, not the 2.1.228 binary under test. Because all three attribution routes are
inside the payload JSON, a design that reads the payload and never the
environment does not inherit that caveat at all.

**`CLAUDE_PID` is the exception, and it was earned rather than assumed.** It is
the agent process's own PID, cross-checked **five times** against the PID the
launcher reported — 320, 9204, 1732, 15152, 12068, each matching its run.
`CLAUDE_PROJECT_DIR` is dropped: never cross-checked, and unnecessary because
`cwd` is in the payload.

**Anything added later that depends on how the agent was launched is still
holding the nested-fixture caveat** and must be labelled as such until
re-established from a plain launch.

### 4. The last event is never terminal, so a third state is mandatory

Measured twice, by killing a running agent and reading what was on disk.

**Killed mid-turn**, with a `UserPromptSubmit` hook still in flight:

```
SessionStart
UserPromptSubmit          <- last written state
```

**Killed mid-tool**, with a `PreToolUse` hook still in flight:

```
SessionStart
UserPromptSubmit
PreToolUse   tool=Read    <- last written state
```

In both cases: **no `Stop`, no `SessionEnd`**, process confirmed gone. For
contrast, a normal completion writes `Stop` and then `SessionEnd` about 150 ms
apart.

The window was made deterministic by a hook that writes its line and then blocks,
because three attempts at a naturally long turn completed too fast — recorded
because it is how the state was produced, and because the mechanism of the block
does not change what was written.

**The consequence is the whole design constraint:** *last reported state* is not
*current state*. An agent whose last event is `PreToolUse` may be running a slow
tool, waiting on approval, or dead, and **the hook record cannot tell those
apart** — the last event of a crashed agent is indistinguishable from the last
event of a busy one. Rendering the last event as the current state re-invents
exactly the value hooks were adopted to stop inventing; it is constraint 5
arriving through the mechanism chosen to avoid it, which is why it is stated here
rather than left to the display.

**So a third state is not a nicety, it is required by the measurement**, and it
must be distinguishable from both "working" and "stopped".

**Round 2 leaves this argument untouched and makes it stronger, and the
distinction matters because §5 below changes a great deal.** *Added 2026-08-17.*
What §5 retracts is a claim about what an unmatched `tool_use_id` *means*; it
does not touch the reason a third state exists, which is that the last written
event is not the current state. **The third state's composition is unchanged.
What changes is how much lives inside it** — round 1 read "alive and quiet" as
one case plus a hedge, and it is three cases with no discriminator. A third state
is required by the same measurement and now has more to hold.

### 5. Liveness is the only measured discriminator, and it is not sufficient alone

`CLAUDE_PID` gives a check the hook record cannot: whether the process that
reported the last event still exists. That is the only measured way to separate
*busy* from *gone*.

**It does not restore certainty, and reading it as though it did would repeat the
error one level down.** A live PID says a process exists, not that the agent is
healthy or making progress; and PIDs are reused by the operating system, so a
live PID is not proof it is *the same* process.

**Pairing the PID with `session_id` does not fix that, and an earlier draft of
this section said it did.** `session_id` comes from the payload — it is static
data written to a file by a process that may no longer exist. It identifies the
*run*; it says nothing about the process currently holding that PID. On reuse,
"PID 320 is alive" is a true statement about something else entirely, and no
amount of payload data touches it. **So "process alive" is itself a potentially
invented value, sitting inside the fact that is supposed to keep the rest
honest.**

**The fix is composite identity — `(pid, start_time)` — where a mismatched start
time means the PID was recycled.** Its cost is genuinely cross-platform work and
is stated rather than waved at:

- **Linux:** field 22 of `/proc/<pid>/stat`. No dependency.
- **macOS:** `sysctl` `KERN_PROC_PID` → `kinfo_proc`. `libc` plus `unsafe`.
- **Windows:** `GetProcessTimes`. Measured available on this machine at 100 ns
  resolution and stable across reads — three identical reads of one process —
  and **not universally readable**, because the datum is access-controlled.
  *"Start time unavailable"* is therefore a third outcome and must not collapse
  into either alive or gone. That is the same `NotAttempted` line one level down.

  **The observable was wrong, and it was the instrument's.** *Corrected
  2026-08-17.* This bullet read that *"one process in a four-row sample returned
  an empty start time"*, and an empty start time is not what the syscall does.
  Re-measured through raw `OpenProcess` + `GetProcessTimes` with a positive
  control on our own process:

  | PID | class | outcome | Win32 |
  | --- | --- | --- | --- |
  | own | **positive control** | OK, 100 ns FILETIME | 0 |
  | a user-launched `node` | **the subject class** | OK | 0 |
  | 4 | `System`, protected | `OpenProcess` returns NULL | **5** `ACCESS_DENIED` |
  | 0 | `Idle` | `OpenProcess` returns NULL | 87 |
  | 999999 | nonexistent | `OpenProcess` returns NULL | **87** |

  The empty string came from **.NET's `Process.StartTime` accessor**, which on a
  protected process yields an empty value **and raises nothing** — a blank cell
  in a table, no error to notice. The syscall is loud. Suspicion goes to the
  instrument first, and it was the instrument again; ADR-0002 §7 carries the
  general form and explicitly does not fold this into the six-for-six tally,
  which is a different population.

  **So the arm is reachable, and PID 4 is a real fixture — with the claim it
  supports stated narrowly.** It exercises the real syscall path to a real
  unavailable outcome, so no synthesised value is needed on Windows. It does
  **not** establish that the subject can ever be in that state: the subject is
  always a user-launched `claude`/`node` process, measured `OK`. The fixture also
  depends on two facts about the environment rather than about the code — that
  PID 4 is the protected `System` process on build 19045, and that the session is
  **non-elevated**. An elevated run holding `SeDebugPrivilege` may not reproduce
  it; that was not tested. **Both belong in a comment beside the fixture**, or
  the day it stops failing nobody will know why.

  **And the real hazard is one this bullet did not have: `OpenProcess` returning
  NULL is two outcomes wearing one observable.** `ACCESS_DENIED` means *the
  process exists and cannot be read*; `ERROR_INVALID_PARAMETER` means *there is
  no such process*. Code that branches on the null handle collapses them, and it
  collapses them in the direction that reads as reassurance: **a live agent whose
  start time is unreadable renders as stopped.** That is ADR-0010 §5's
  *"nonzero means not-ignored"* one platform down — an error read as a clean
  answer — and §9 carries it as a requirement on the diff with its paired
  control. Branch on `GetLastError`, never on the handle.

- **Linux and macOS: unmeasured, and the asymmetry is therefore unknown.**
  Round 2 ran on Windows only. Whether either can produce a real *unavailable*
  through its own route — `/proc/<pid>/stat` field 22, `KERN_PROC_PID` — is not
  established, and if they cannot, the platform limit inverts ADR-0010 §10's,
  where Unix had the reachable fixture and Windows had the synthesised one. That
  inversion is worth recording **if it is measured**; it is not inferred here.
- **Or one crate covering all three**, which is a dependency added to a library
  every embedder links — the cost ADR-0008 §4 weighed when it declined 18 crates
  for a TLS stack. Not weighed here, because that is a decision for the diff.

**Without something of that shape, liveness is an inference wearing the costume
of a fact**, and it belongs in the same category as the silence this whole design
refuses to read.

**How much is actually unresolved while a process is alive and quiet — the
narrowing is RETRACTED, and nothing replaces it.** *Amended 2026-08-17, from the
2.1.233 round. The retracted sentence is quoted rather than deleted, because a
retraction a reader cannot see is one they will re-derive.*

It read: *"an unmatched `tool_use_id` with the reporter alive is a tool in flight
— a reported fact, not an inference from silence, and it is not thinking."* It
also conceded one open distinction, conditional on something recorded as
unmeasured: whether approval is requested inside the tool window.

**Both halves were measured. The condition resolves to the bad branch, and the
sentence is false for a reason that was not on the list at all.**

Three fixtures, each with its precondition constructed rather than hoped for —
round 1's *"headless auto-denies"* did not reproduce, because the child inherited
a permission state that simply allowed the tool and nothing ever asked. That run
proved nothing and is recorded as the unreached guard it was. `permissions.ask`
in the fixture's own settings is what produced a real decision.

1. **Approval sits INSIDE the window.** A blocking MCP `--permission-prompt-tool`
   held a real approval for **9 s**: `PreToolUse` opened `toolu_01JtAH…` at
   3387 ms, the prompt was asked at ~3400 ms and answered at ~12400 ms, and
   `PostToolUse` closed the same id at 13548 ms. Two snapshots taken *inside* the
   wait each report exactly one open `tool_use_id` — the one being waited on —
   and no other events. **In the hook record a 9 s approval wait is byte-identical
   to a 9 s slow tool.**

2. **A denied tool leaves its `tool_use_id` open permanently.** Same fixture, the
   decision refused: `PreToolUse` opened `toolu_01KJH…` at 3046 ms, **no
   `PostToolUse` ever**, `PostToolBatch` at 3143 ms, then `Stop` at 8170 ms and
   `SessionEnd` at 8305 ms. The subject's own report confirms a real denial
   (`permission_denials` non-empty, `is_error=true`). **The id is not open until
   it finishes. It is open by definition, in a session that has stopped.**

3. **`PermissionRequest`, `PermissionDenied` and `Notification` did not fire** in
   any of it, with the names demonstrably registered (§2) and the mechanism
   demonstrably run. Positive control: 16 and 18 hook entries from the same
   config in the same runs, including `PreToolUse` for the very tool whose
   permission was prompted.

**The retraction is total rather than partial, and that is a decision rather than
an oversight.** The narrowed form available here is *"either a tool is executing,
or the agent is waiting for approval, or a tool was denied and nothing is
running"* — a three-way disjunction over states with no common consequence.
**That is not a reported fact; it is an absence of information with a list
inside it**, and writing it as a narrowing would leave this section claiming to
have bounded something it has not. So: **an unmatched `tool_use_id` with the
reporter alive supports nothing about the present.** The gap §5 opened with is
open, and it is wider than when it was written.

**Case 2 is the worst of the three, and the general shape it exposes is worth
more than the case.** §7 records that absence of events does not prove absence of
work — an empty monitor looks like a quiet machine. Case 2 is the converse:
**presence of an unclosed event does not prove presence of work.** An
unmatched id survives `Stop` and `SessionEnd`, so it is not even evidence the
session is running.

**The channel is ambiguous in both directions, not one.** That is the sentence to
carry forward, because every mechanism in this document so far was built against
the silent direction — the wiring proof, the liveness check, the third state —
and a design that guards only silence is guarding half of the problem. Silence
overstates stoppage; an open event overstates activity. Constraint 5 has two
faces here and they invent in opposite directions.

**`PostToolBatch` is a candidate closer, filed with its bound and not as a
replacement.** It fired in both runs — 97 ms after the denial that produced no
`PostToolUse`, and 113 ms after the `PostToolUse` that did — so it closed the
tool phase in the case where `PostToolUse` never came. **Two observations, both
single-tool batches, semantics under parallel or nested tool calls unmeasured.**
It does not rescue the retracted sentence and must not be written up as though it
did; what it is, is the first thing to measure when this is picked up.

**Where the negatives in point 3 stop.** They are headless, through the MCP
permission route. Whether the **interactive TUI** path fires them is unmeasured:
no pty is constructible in this environment, which is the identical one-sided
limit ADR-0005 §10 records for the pager, from the same cause. So §8's first item
is answered on one path and open on the other, and the honest form of the result
is *"not on this path"* rather than *"not used"*.

### 6. What must be true of the display, whatever it looks like

- **The three-way distinction survives into the UI.** ADR-0009 §3c already
  requires that a fact we did not establish renders differently from a fact we
  established as absent. Here that is: *reported working and reporter alive*,
  *reported working and reporter gone*, and *reported stopped* — with the middle
  one never borrowing the appearance of either neighbour.
- **A timestamp is part of the fact, not decoration.** A reported state is a
  fact *as of* an instant, and one without its age is a claim about now.
- **Absence of events is not a state.** An agent that never ran and an agent
  whose hooks were never installed both produce an empty record, and neither is
  "stopped". This is the same `NotAttempted`-versus-`NoEvidence` line ADR-0003
  draws and ADR-0010 §6 applies to shadowing.
- **A project the registry has never seen is a third thing, not a missing
  project.** *Added 2026-08-19 with §7b's user-level install.* A user-level hook
  fires **everywhere**, so records arrive from directories vibe was never told
  about — which is new: everything else vibe reports is about a project it was
  given. They render as **unregistered**. Not dropped, because the events are
  real and the session is named in every payload. **Not attributed to a nearest
  match**, because that invents the fact deciding which project a record belongs
  to. The payload carries `cwd`, so *where* is known; what is unknown is which
  registry entry, if any, it corresponds to — and *unknown* is the word for it.
- **A hook that was suppressed must not render as an agent that was quiet.**
  *Added 2026-08-19.* §7b leaves `if` as a residual omission-dependency whose
  failure direction is suppression, and `matcher` can suppress a whole group.
  A suppressed hook delivers nothing, which is the same observable as an idle
  agent and as a hook that was never installed. All three are **unknown** here,
  and the display may not resolve them by preferring the reassuring one.

### 7. Vibe may write hook config, as one explicit act — and the wiring carries its own proof

**Decided.** An earlier draft left this open on the ground that hooks create a
consistency relationship between two artifacts, which is the shape ADR-0010 §3
rejected for prompts, and doubted whether the reasoning transferred *because the
second artifact is configuration rather than content*. **That was the wrong
distinction.** ADR-0010 §3 did not reject writing; it rejected **sync** — a
standing obligation to keep two things equal, discharged silently and repeatedly.

So the permitted shape is the one ADR-0010 §3 already permitted itself: **an explicit
`vibe monitor install` — singular, user-initiated, never automatic.** What must
not exist is silent repair of drift.

**And drift is not answered by syncing; it is answered by versioning the
contract.** The installed hook declares which contract version it implements,
vibe reads that declaration, and **a mismatch is reported, never repaired.** A
repair would be sync under another name, and it would do it at the moment the
user is least able to see it.

**The larger hazard is not the write, and it outranks it: silent
non-delivery.** If the hook is missing, misconfigured, broken, or removed, vibe
receives nothing — and **"no events" is indistinguishable from "the agent is not
running"**. That is constraint 5 at the centre of this feature, in its worst
form, because the default reading is the reassuring one: an empty monitor looks
like a quiet machine.

**Therefore the governing constraint is not about writing at all: the wiring must
carry its own proof, per session.** A session that has delivered a `SessionStart`
has demonstrated its wiring **live, for that session** — the hook ran, the
transport worked, the payload arrived. A session with no such marker is
**unknown**, never idle, and no amount of subsequent silence upgrades it.

This is `VIBE_REQUIRE_GH`'s shape (ADR-0002 §7) applied to the channel rather
than to a control: the *result* carries the evidence that the mechanism ran, so
nothing has to be remembered or checked on the side.

**The proof is stronger than `SessionStart`, and the strengthening does not help
where it matters. Both halves are recorded, because either alone misleads.**

- **Stronger:** *every* delivered event proves the channel live **at its own
  timestamp**, not merely at session start. A hook removed mid-session stops
  refreshing the proof, so the uncovered window is **"since the last received
  event"** rather than "since session start" — a much smaller and
  self-narrowing hole, which any traffic at all closes again.
- **And useless exactly where it is needed:** that window is precisely the
  *silent* one, which is when the question is being asked. Traffic refreshes the
  proof; **absence of traffic is simultaneously the question and the hole.** An
  agent that has said nothing for an hour is the case a monitor exists for, and
  it is the one case where the freshness of the wiring proof is at its weakest.

So the limit stands in its sharpened form: **a delivered event proves the wiring
worked when it was delivered, and nothing about now.** That is the
environment-shaped hole closed and the code-shaped one left open, exactly as
ADR-0002 §7 records for the original — and it is why §5's liveness check is not
redundant with this one. They fail in different directions, which is the only
reason to have both.

It follows that **monitoring is opt-in per project** — hooks fire only where
installed — and that an uninstrumented project renders as unknown rather than
idle, per §6.

**The file vibe would write is strict JSON, measured rather than assumed.**
*Added 2026-08-19 on 2.1.234, because the shape of the write decides what an
editor has to preserve.* Three variants were planted separately and each is
rejected as *"Invalid or malformed JSON"*: a `//` line comment, a `/* */` block
comment, and a trailing comma. Paired against a strict file in the same fixture
shape, which produces no such report. **So the artifact `toml_edit` exists to
protect for manifests — a user's comments — cannot be present in this file at
all**, and what an editor here must preserve is narrower: key order, whitespace,
and any key vibe does not own.

**What such a file looks like in practice — and the sample is n=1 in cause,
not n=3.** *Corrected 2026-08-19.* Three real files were found, every one
2-space indented, LF, no BOM, strict JSON, 8 to 14 lines. **They do not agree
about formatting; they share a writer.** `~/.claude/settings.json` holds
`model`, `enabledPlugins` and `effortLevel`, which Claude Code's own `/config`
and `/model` write. A project's `settings.local.json` holds
`permissions.allow` entries carrying machine-escaped inner quotes
(`\(Get-Content \"C:\\Users\\...`) that nobody types. Both are
**tool-generated**, in the same serialiser's format. The third is a vendored
crate's file in the same permissions shape, with no history available to say
which. So *"all three are 2-space"* is one cause observed three times, and it
establishes **almost nothing about a file a person formatted**.

**None of the three declares `hooks`, so none of them is the file install
actually has to survive.** That file is a **re-install**: a `settings.json` that
already contains a vibe hook. Constructed and measured — 49 lines, user keys
(`model`, `effortLevel`, `permissions`, `env`), a user's own `PreToolUse` hook
under a `matcher`, and a vibe `SessionStart` hook in the `args` exec form.
Accepted by `claude doctor`. Round-tripped through `serde_json` with
`preserve_order` and `to_string_pretty`:

| the same content, formatted as | lines rewritten, of 49 |
| --- | --- |
| 2-space | **0 — byte-identical** |
| 4-space | **44** |
| tab | **46** |

**So the declared limit is that table, not a shrug.** A 2-space file survives
untouched; a 4-space or tab-indented file is effectively rewritten whole. Which
one a user has is not knowable in advance, so the diff is shown before it is
written rather than predicted — constraint 2's `--dry-run`, which makes this an
output rather than a blocker.

**`matcher` lives on the GROUP above the hook, and that makes it an idempotency
constraint rather than a field.** Measured (§2, round 3b): a non-matching
`matcher` suppresses its group's hooks entirely. So **where** install writes
decides whether a matcher vibe never wrote can silence vibe's hook — appending
into an existing group inherits that group's matcher. **Install writes its own
group**, and a re-install that finds a vibe hook inside somebody else's group
has found a configuration it did not create and must report rather than repair.

### 7a. Transport: a file — and `http` chosen, costed, and reversed

*Added 2026-08-17. This was missing from §8's list of open decisions, which read
as complete and was not.* The omission had a reason and the reason was wrong:
transport looked like a decision for the diff, by analogy with §5's
cross-platform crate. **It is not analogous.** A crate choice is a dependency;
transport decides **whether non-delivery is detectable at all**, and
non-delivery is §7's central hazard. A file append and a socket fail
differently. That is §7's own question, not an adjacent one.

**Decided: a file, appended to by a `command` hook in the `args` exec form.**
`prompt` and `agent` cost a model turn and are refused on the ground ADR-0010 §6
refuses enrichment as a base layer. `mcp_tool` needs a server per session and
reports failure to the agent rather than to us. `http` was **chosen first, and
the choice did not survive being costed** — the reversal is recorded below in
full, because it is more useful than the conclusion.

#### Why `http` was chosen, and why that property did not hold up

The argument was real and no other variant has it: **`http` is the only variant
where the receiving side holds independent evidence of its own liveness**, so
*"no events"* splits into *"the receiver was up and heard nothing"* and *"the
receiver was down"*. A file cannot make that split — a file that stops growing is
indistinguishable from a quiet agent, which is §7's central hazard exactly.

Costing it produced a narrowing, and following the narrowing reversed it.

**1. The narrowing.** The receiver's evidence covers only the interval it has
been **continuously up**. Across a restart it cannot separate *"nothing happened
while I was down"* from *"things happened while I was down"*, because **a process
that is down cannot record its own downtime.** The repair is for the receiver to
record its own coverage intervals durably, so *"I have no coverage for this
window"* is a reported fact — §7's proof-carrying shape applied to the receiver.
That needs durable state regardless, so `http` does not replace the file; it puts
a listener in front of one.

**2. Give both transports that coverage log, and the residual is identical.**
Nothing arrived because the agent was quiet, or because the hook is broken.
Neither transport touches that, and **the per-session wiring proof of §7 handles
it identically in both.** So the coverage log equalises them on the only question
either was being chosen to answer.

**3. And then the asymmetry runs the other way.** *"Receiver down"* is a failure
mode **that exists only because there is a receiver.** A file has no up or down
state, so the question never arises. During receiver downtime the hook still
appends to a file; with `http` those events are **lost**. Sleep, reboot and crash
are precisely when vibe is not running — so `http` loses events exactly on the
**long-silence case the monitor exists for**, which is the case §7 already
identifies as the one where every other proof here is weakest.

**4. So the deciding property was information about the receiver, not about the
subject.** Its only use is accounting for a loss the file transport does not
incur. **Information whose sole purpose is explaining your own failure mode is
not a reason to adopt the failure mode**, and that sentence is the transferable
part of this reversal.

**The reversal was falsification-tested rather than argued to a stop**, because
the property being discarded was genuine. The question put was: *name a case
where `http`'s receiver holds information a file cannot, once both have a
coverage log.* Seven candidates were tried and each is symmetric:

| Candidate | Why it is not an asymmetry |
| --- | --- |
| sink unwritable | file-unwritable ↔ receiver-unreachable; both lose silently |
| arrival timestamp | the hook stamps its own time in both, and it is one machine, so no skew |
| synchronous acknowledgment | informs the *agent*, not vibe; `asyncRewake` and exit `2` are open to a `command` hook too |
| session discovery | the same set, learned at read time rather than at arrival |
| malformed payload | lands in the file exactly as it lands at the receiver |
| record deleted or truncated | the receiver's coverage log is itself a file with the same exposure |
| real-time push | latency, not information; a watcher exists on all three platforms |

**None survived, and the failed falsification is what licenses the reversal.**
Had one survived, the decision would have stood. Recorded as a list rather than
as a conclusion so the next reader can attack the same question with a candidate
nobody here thought of, which is the only way this gets overturned again.

**What `http` would still have cost, kept because the costs are what made the
narrowing worth chasing:** vibe grows a long-running mode it has never had —
every command today is one-shot; an inbound socket is a channel *added* to an
environment, which is the opposite of the move ADR-0005 §10 and ADR-0008 §4 are
built on, and a local port accepting hook payloads is one **anything on the
machine can post false events to**, an integrity problem for a tool whose whole
product is reported facts; and `headers` plus `allowedEnvVars` would make it a
secret-management problem in a tool that deliberately has none (ADR-0008 §5).
Those costs did not decide it. **The reversal is on the deciding property
failing, not on the costs outweighing it** — which matters, because a decision
reversed on cost is re-opened by cheaper hardware and this one is not.

#### What the file transport is therefore required to carry

These are **parts of the design, not cost lines**, and they are written as
requirements because the reversal moved them from *"a price `http` avoids"* to
*"work this transport must do"*.

- **One file per writer — `(session_id, agent_id, declared writer identity)`.**
  *Decided 2026-08-17 as `(session_id, identity)`; **agent_id added 2026-08-18**
  after measurement showed the two-part key broken.* Multiple sessions, and the
  duplicate delivery §2 measures, would otherwise write one sink at once.

  **The guarantee is one writer per `(session, agent, identity)`. That is
  weaker than "concurrent append cannot happen", and the gap between those two
  sentences is exactly the unmeasured case.** Written first and in this order
  because the label reaching one step past the mechanism is the failure §5's
  genealogy records four times, and this is the fifth opportunity.

  **What broke the two-part key, measured rather than reasoned.** A subagent
  **shares its parent's `session_id`** — every subagent-owned `PreToolUse`,
  `PostToolUse` and `PostToolBatch` carries the parent's, and three subagents in
  parallel still produced exactly one `session_id`. So `<session>__<identity>`
  puts the parent and every child in one file. That is not an inference from the
  shared id: **12 pairs of hook processes with the same declared identity were
  observed alive at the same time**, overlaps of 1.1 ms to 61.2 ms, measured by
  recording each hook's process lifetime rather than a write timestamp.

  **`agent_id` is what separates them**, and it is a payload fact: present on
  `SubagentStart`, `SubagentStop`, and on every subagent-owned tool event;
  **absent on parent-level events**. All 12 observed overlapping pairs are
  cross-agent, so the three-part key eliminates every instance that has been
  seen.

  **The one thing that would kill this repair is intra-agent concurrency, and it
  is UNMEASURED.** *Amended 2026-08-19 — see the round-3 paragraphs below. It has
  now been measured on one fixture and did not appear, so this word reads **not
  proven impossible** rather than **never looked at**. Left in place because the
  paragraphs that follow are the argument for the limit and they were written
  against it.* If two hook invocations for a single agent can overlap — two
  tool calls in flight in one turn — they share `session_id`, `agent_id` *and*
  identity, and no key drawn from the payload can separate them, because the
  discriminator would have to be per-invocation and the payload has no
  per-invocation field.

  **What was tried, recorded as attempts rather than as a negative result:**

  1. A prompt for six independent file reads in one turn — the agent emitted
     **six assistant messages with one `tool_use` block each**.
  2. The same, plus `--append-system-prompt` mandating parallel blocks in one
     message — identical result.
  3. The settings schema and the CLI searched for a parallel-tool-call lever —
     **none exists**.

  Maximum `tool_use` blocks in one message across every run: **one**. So no batch
  ever existed and the zero is a property of the fixture. The instrument was
  demonstrably able to detect the hazard — 151 overlapping pairs overall, the
  smallest at **1.1 ms** — which is what makes the zero reportable as *not
  constructed* rather than as *absent*.

  **Shipped as a declared limit rather than waited on**, because the precondition
  is model behaviour and not a feature with a flag, so it may never become
  constructible. If it fires, the escalation shapes are already named below and
  neither should be rediscovered as novel: **a file per record**, which is how
  maildir avoids locking entirely, or **the framed shared file**, rejected here
  because it detects rather than prevents and the torn record's contents are
  still lost.

  **THE PRECONDITION IS CONSTRUCTIBLE, AND CONSTRUCTING IT DID NOT PRODUCE THE
  HAZARD.** *Measured 2026-08-19 on 2.1.234.* Both halves are recorded because
  the first retires *"it may never become constructible"* and the second is the
  only reason the limit still stands.

  **The zero above was a property of the fixture, and the lever is the subagent's
  own prompt.** The three attempts recorded above put the parallel instruction in
  the *parent's* prompt and in `--append-system-prompt`, and the maximum stayed
  at one `tool_use` block. Instructing the **subagent**, inside the prompt the
  parent passes to it, produced **six `Read` calls in one message from one
  agent** — visible as a single `PostToolBatch` whose `tool_calls` array holds
  six entries, all carrying the same `agent_id`. A two-call batch appeared
  earlier the same day without being asked for. So the sentence to keep is not
  *"no batch exists"* but *"the fixture never built one"*, which is ADR-0010 §5's
  rate again.

  **What replaced it is a property of a BUILD, not a property of the design,
  and the register entry says so.** The distinction is the whole finding:
  *"measured and held"* reads as something this shape guarantees, and it is not.

  > **Intra-agent same-key concurrency.** The precondition is constructible —
  > the parallel instruction in the subagent's own prompt, six `Read` calls in
  > one message. On `anthropic.claude-code-2.1.234` at
  > `.vscode/extensions/anthropic.claude-code-2.1.234-win32-x64/resources/native-binary/claude.exe`,
  > **hook invocations under one key serialise**: smallest observed same-key gap
  > **101.6 ms** against a 60 ms recorded window, **zero** overlapping pairs,
  > across 48 invocations spanning 4.9 s. Positive control: a **same-key**
  > synthetic pair, four invocations sharing one
  > `(session_id, agent_id, identity)`, **6 overlapping pairs at 39–59 ms**, on
  > both instruments. One fixture. **Both dependents retained** — the file key
  > and the non-decreasing-stamp check — because serialisation is a measured
  > property of one build and not a property of the design.

  **The mechanism the asymmetry points at, stated as the reading rather than as
  a finding:** this build appears to serialise hook invocations **within** an
  agent and not **across** them. Six parallel tool calls produced six
  invocations spread over 4.9 s, while the same run carried 24 cross-identity
  overlaps at 43–58 ms. *"Six tool calls in one message"* is a different shape
  from *"two hook processes alive under one key"*, and the 4.9 s span says the
  second shape did not arrive — the precondition was built and the hazard's own
  shape was not.

  **The positive control had to be the same shape as the target, and the first
  one was not.** Cross-identity overlaps prove the analyser can pair invocations
  **between** files; they say nothing about the grouping path the same-key
  answer travels, and a bug in that path produces exactly the observed picture —
  cross-identity at 43–58 ms and a same-key zero. So the control spawns
  invocations that **share one key** on purpose
  (`scratchpad/hook-overlap-control.js`) and the analyser must report them as
  same-key. It also **refuses rather than retries**: if the spawns land too far
  apart to overlap it exits nonzero and says the control did not fire, because a
  control that loops until it succeeds cannot tell *"the instrument works"* from
  *"the twentieth attempt got lucky"*. That is not hypothetical — the first
  same-key control was a shell backgrounding two `node` calls, it fired at
  30.2 ms once, and re-running the identical setup produced **no overlap at
  all**. A control decided by interpreter startup is a coin flip that reads as a
  control.

  **And a zero was withheld before any of this.** A first collector recorded
  only the microseconds around its own work — 0.02 to 0.10 ms — and reported no
  overlap **of any kind**, including pairs known to be concurrent. Withheld
  under ADR-0002 §7's channel rule rather than published. The repair is a
  disclosed 60 ms dwell, which cannot manufacture an overlap under serial
  dispatch, plus a second instrument that asks the same question with no clock
  at all — a marker file created at entry, removed at exit, and the directory
  listed while the process is alive. The two agree exactly on every count
  reported here.

  **The bound runs one way and it is the safe way.** The recorded interval
  starts when node starts, which is after the OS created the process, and ends
  before exit rather than at it, so it is a **subset** of the true lifetime: a
  detected overlap is real, and a zero is evidence rather than proof. What moves
  the dependents is an overlap on some fixture, or a build whose dispatch is not
  this one — not another zero on this one.

  **What the cross-identity overlaps confirm, since they were free:** §2's
  duplicate delivery is not merely two records, it is **two hook processes alive
  at the same time**, on every event, for the same session and the same agent.
  They are separated by nothing except the identity each hook declares — which is
  precisely the filename component this design puts there, and the reason the key
  cannot be *"the settings file it came from"* or anything else the payload
  carries.

  **TWO things rest on this limit, not one, and the second was found by reading
  rather than by measuring.** *Added 2026-08-18, while building the reader.*
  Recorded here rather than beside the reader, so anyone re-deciding the limit
  sees everything that would move with it:

  1. **The file key.** `(session, agent, identity)` is one writer only while a
     single agent cannot have two hook invocations in flight.
  2. **The non-decreasing-stamp check**, three bullets down — *"within a single
     file there is exactly one writer, so stamps must be non-decreasing; a
     decreasing stamp inside one file is direct evidence the clock stepped."*
     That sentence quietly reads *one writer* as *one process*. It is not: a file
     is appended by **many invocations of one hook**, each a separate process
     reading the same wall clock. The stamps are only ordered because the
     invocations are sequential — which is the same unmeasured assumption.

  **So a decreasing stamp is direct evidence of a clock step only under the
  limit.** If intra-agent concurrency exists, two overlapping invocations can
  stamp out of order with a perfectly healthy clock, and the check reports a
  fault that is not there — the inverse of this document's usual failure
  direction, and still a wrong claim. The reader therefore treats it as an
  **observation that reorders nothing**, which is correct either way and is the
  only reading that survives the limit being wrong.

  **The general shape is worth more than the instance:** a declared limit gets
  cited once, at the decision it was declared for, and then quietly acquires
  dependents as later work leans on the same assumption in different words.
  *"One writer"* and *"one process"* were the same phrase here until someone
  needed the second meaning. When a limit is declared, the obligation is to
  re-check it at every later use rather than to inherit it — which is ADR-0008
  §6's *re-established rather than inherited* applied to an assumption instead of
  to a control.

  **The costs the third component carries, none of them new and all of them
  now owed:**

  - `agent_id` becomes a path component, so it takes the **same charset, the
    same normalisation, and the same validation at install and at write** as the
    identity.
  - The `session_id` gap recorded above **doubles** — two payload-sourced values
    reaching a filename rather than one.
  - Observed `agent_id` values are 17 lowercase alphanumerics
    (`ab8b50189992e6091`), which the charset accepts. **That is a sample of
    seven, not a guarantee about the field**, and a value outside the charset is
    refused rather than assumed impossible.

  **`agent_id` is absent on parent events, and absence is encoded structurally
  rather than by a reserved word.** A literal such as `root` could collide with
  a real `agent_id`, and nothing measured bounds that id space. Instead `__` is
  **forbidden inside every component**, which makes the separator unambiguous
  and lets the component *count* carry the distinction: two components is a
  session-level record, three is an agent-level one. Nothing has to be reserved
  and no collision is representable.

  **The key said `settings source` for half a day, and that was wrong.**
  *Corrected 2026-08-17.* It was read off a fixture in which each settings file
  declared exactly one hook per event, so one source meant one writer. That is a
  property of the fixture. Measured from the schema: the per-event value is an
  **array of matcher-groups, each holding an array of hooks**, so one settings
  file can declare N×M command hooks for a single event. All of them share a
  source, all of them would open one file, and **concurrent append returns inside
  the design whose entire justification is that it cannot occur.** The identity
  must therefore be the one the hook itself declares, one per installed hook, and
  never a property of the file it was declared in.

  **Two shapes were rejected and the first was inadmissible rather than worse.**
  A shared file relying on append atomicity has a failure — a split or
  interleaved record — that **cannot be induced on demand**: it depends on
  filesystem, OS and timing, and may never reproduce on local NTFS or ext4. A
  failure that cannot be produced deliberately **cannot have a paired control**,
  and a guard without one has never been accepted here, so it is out before cost
  is discussed. A shared file with framed records is admissible — framing makes a
  torn record certainly detectable rather than heuristically so, and its fixture
  is fully constructible because the hazard moves into the *reader* — but it
  **detects rather than prevents**, and the torn record's contents are still
  lost.

  **What decides it is neither of those.** Both shared-file shapes rest on an
  atomicity guarantee **this project cannot measure on two of its three
  platforms** from the machine it develops on. That is the same shape as the
  `PIPE_BUF` assumption that died on contact with measurement (ADR-0002 §7): a
  cross-platform property taken on authority. **One writer per file does not
  handle that dependency, it removes it** — the guarantee stops being load-bearing
  because nothing concurrent happens. Same technique as the missing
  `FileOp::Delete` and ADR-0005 §10 rule 1's closed enum.

  **The positional bound is the bonus, not the reason.** With one writer,
  interleaving is impossible and only truncation remains — a crashed hook, a full
  disk, a killed process — so **only the last record in a file can be partial.**
  The reader therefore validates a *tail* rather than scanning for corruption
  anywhere, which is the whole of the framing that survives from the rejected
  shape.

- **Ordering across files is authored, not observed, and the reader must be able
  to refuse.** *This is the largest cost D carries and it is recorded before it
  becomes an implementation detail.*

  **Measured 2026-08-17: no event type carries a timestamp.** Across all eight
  observed types the payload offers `session_id`, `prompt_id`, `turn_id`,
  `message_id`, `tool_use_id`, `index` and `duration_ms` — and nothing naming a
  point in time. Every timestamp in this round's data was added by the
  measurement harness. So under D, where history is reassembled from several
  files, **the merge key is a value the writer invents.**

  That is constraint 5 pointed at *ordering* rather than at a value, and it is
  the more dangerous target: a wrong value is a wrong field, while a wrong order
  is a **plausible history**. Clock skew between hook processes, or a wall clock
  stepping backwards under NTP, silently reorders events into a sequence that
  reads perfectly.

  **What the payload does support, at no cost and with no clock, is a partial
  order**: `session_id` groups, `prompt_id` groups a turn, `tool_use_id` pairs a
  `PreToolUse` with its `PostToolUse`, `index` sequences `MessageDisplay` deltas
  within a message, and the lifecycle constrains the ends. **That is a payload
  fact and it is the primary ordering.** Authored stamps are the fallback, used
  only where the payload orders nothing.

  **So the reader's contract is a partial order, not a sequence** — and where two
  records are unordered by both the payload and the stamps, it must **say so
  rather than present one**. That cost propagates: it reaches §6's display and
  ADR-0009's constraints as a third state one level down, *ordered* versus
  *unordered with respect to each other*, with the second never borrowing the
  appearance of the first.

  **One check D makes available and a shared file would muddy:** within a single
  file there is exactly one writer, so stamps must be non-decreasing. **A
  decreasing stamp inside one file is direct evidence the clock stepped** — a
  reported fact rather than an inference, detectable with no cross-file
  reasoning at all.

  **Deliberately not taken yet: a monotonic stamp.** A boot-relative monotonic
  clock does not step backwards and is understood to be comparable across
  processes on one boot, which would give a total order within a session — and
  *"understood to be"* is exactly the standing this section is not going to build
  on again. It is **unmeasured on all three platforms**, and the last
  cross-platform property accepted on that standing was `PIPE_BUF`. It becomes
  available when measured, and not before.
- **The contract version pins the hook's execution properties, not just its
  payload shape.** §7 requires the installed hook to declare which contract
  version it implements. Hooks carry `timeout`, `async` and `asyncRewake`
  (§2), and each changes *whether and when* a record arrives — an `async` hook
  that is killed at session end delivers nothing, and a `timeout` that fires
  truncates a record. **A contract that pins only the payload leaves the
  delivery semantics unpinned**, which is the half that decides whether absence
  means anything. `http` would have inherited this identically; the reversal
  does not reduce it.

  **The three names are not the whole set, and the two that were missing are the
  same class.** *Amended 2026-08-19, from the schema the installed build ships
  (§2, round 3).* A `command` hook carries ten properties — `type`, `command`,
  `args`, `if`, `shell`, `timeout`, `statusMessage`, `once`, `async`,
  `asyncRewake` — and the matcher-group above it carries `matcher`. Of those:

  - **`once`** removes the hook after a single execution. A contract that does
    not pin it admits a hook that delivers exactly one record and then silently
    stops being installed, which is §7's non-delivery hazard with a name in the
    schema.
  - **`if` and `matcher`** suppress the invocation for calls they do not match,
    so records are a **subset** by configuration. Absence then means *filtered*,
    and nothing downstream can tell that from *nothing happened*.
  - **`shell`** decides whether a shell is in the channel at all. §7a's transport
    is the `args` exec form precisely because it spawns directly; a `command`
    without `args` runs through a shell, and that is a different channel wearing
    the same `type`.

  `statusMessage` is presentation and pins nothing. **So the contract's execution
  half is `timeout`, `async`, `asyncRewake`, `once`, `if`, `matcher` and `shell`,
  and the earlier list of three was an enumeration of what had been read rather
  than of what exists** — the same rate §2 records for the event table.
- **Writer identity is declared by the hook, so `unattributed` is a state rather
  than an error.** *Measured: the payload does not name the settings source it
  was delivered through* — §2's two deliveries are distinguishable only by
  something the hook itself supplies. Under D that identity is half the filename,
  so it moves into §7's contract declaration alongside the version.

  **Uniqueness is enforced, not remembered, and the enforcement point is the
  config rather than the records.** A duplicated identity collides silently and
  is exactly the failure this shape exists to make unrepresentable, so a rule
  someone follows is not enough.

  **The writer cannot be the detector, and the obvious design does not work.**
  Creating the file exclusively catches nothing: a writer appends across every
  event of a session, so from the second event onward the file legitimately
  exists, and **an existing file is indistinguishable from a twin's** — exclusive
  creation can only ever fire once, on the first event, when there is nothing yet
  to collide with. Locking would serialise twins rather than detect them, and it
  would reintroduce exactly the cross-platform guarantee this shape was chosen to
  stop depending on.

  **So the check is static and runs in two places, both of which vibe already
  visits.** A duplicated identity is a *configuration* fact — fixed before any
  event fires — so it is decidable without reading a single record:

  1. **`vibe monitor install` refuses an identity already declared**, across both
     settings files. That covers the path vibe controls, at the moment the user
     is doing something deliberate.
  2. **The contract read reports it.** §7 already requires vibe to read each
     installed hook's declared contract version and report a mismatch. That read
     enumerates identities at the same time and reports a duplicate as a
     configuration fault. **This is what covers hand-installed hooks**, which §7
     permits and which install therefore never sees.

  **Note this is not detection downstream at read time.** The distinction is what
  is being read: inspecting *records* to notice a collision arrives after both
  writers have written and after the damage. Inspecting the *config* is a check on
  a static property that can run before anything is written at all, and it is the
  only place a hand-installed collision is visible.

  **But the second check is not guaranteed to run first, and that window is
  declared rather than closed.** The contract read happens when **vibe runs** —
  `vibe monitor install`, and whatever command displays monitoring state. Hooks
  fire when the **agent** runs, which is not the same schedule and is usually the
  earlier one. So a hand-installed twin can deliver **for as long as nobody runs
  vibe**, which may be days, and the first read discovers a collision that has
  already been corrupting a file since it was installed.

  **The honest form is therefore a bound on the records, not a guarantee about
  the config**: when the contract read finds a duplicate, everything that
  identity wrote is suspect **back to the start of the affected sessions**, not
  from the moment of discovery. The reader must say that rather than reporting a
  fault from now on, because a collision found today says nothing about when it
  began.

  **What would close it is an install-time-only design** — refusing hand-installed
  hooks altogether — and that is declined for the reason §7 admits them in the
  first place: the hook config belongs to the user, and a tool that only works
  with config it wrote itself has taken ownership of a file it does not own. The
  window is the price of that, and it is stated here rather than left for someone
  to discover that *"the contract read reports it"* meant *"eventually"*.

  **A hand-installed hook that omits it writes a file the reader cannot
  attribute**, and this is the ordinary case rather than the exotic one: §7
  permits hooks installed by hand, so the field will be missing somewhere. The
  reader therefore **lists such a file as `unattributed` and reads its records**
  — the events are real, the session is named in every payload, and only the
  *source* is unknown. Discarding it would lose real events; guessing a source
  would invent one; calling it an error would say something failed when nothing
  did. It is `NotAttempted`'s neighbour: a fact vibe does not have, named as
  missing, with everything that does not depend on it still usable.

  **The one thing it costs is dedupe.** §2's duplicate delivery is identified by
  source, so an unattributed file's records cannot be matched against their twins
  with certainty. That is a bounded, statable degradation — *this session's
  duplicates may not be collapsed* — and it must render as such rather than as a
  session that emitted twice as many events.

- **A failed write exits `1`, never `2`, and never panics.** *Decided
  2026-08-18, when the writer landed.*

  A hook that cannot write is silent non-delivery **arriving from our side** —
  §7's central hazard produced by the mechanism installed to prevent it — so it
  may not be swallowed. That settles loudness to *us*; it does not settle
  loudness to the *agent*, and the two are different questions.

  Exit `2` is visible to the agent and can interrupt the turn. **The monitor is
  additive: vibe works without it.** A hook that stops the user's work over its
  own write failure has inverted the relationship between observer and observed,
  and **an observer that can stop the subject is not one.** Exit `2` buys
  immediacy, and the user pays for it mid-turn.

  **The loss is not silent under `1` either**, which is what makes the trade
  cheap rather than a concession: `WriteOutcome::Failed` carries the stage, the
  `ErrorKind`, the raw OS code and the torn-byte count, so the next read reports
  it. Immediacy is the only thing given up, and it is given up in the one
  direction where the cost lands on someone who did not ask for a monitor.

- **The sink lives where vibe manages it**, not in a directory a user cleans up,
  since an append into a deleted inode succeeds silently on POSIX.

- **Its path is DECLARED at install and passed in argv, never resolved from the
  environment.** *Decided 2026-08-18, when the hook's `main` was built.*

  The obvious implementation is `ProjectDirs`, which is what
  `agents::default_store_path` uses. It resolves from the **environment** —
  `LOCALAPPDATA`/`APPDATA` on Windows, `XDG_*` on Unix — and the hook is a
  **child process of somebody else's tool**, whose environment §3 measured as
  contaminated by its parent's. This design's whole attribution story is *read
  the payload, never the environment*; resolving the sink there would put the
  one thing that decides **where every record goes** on the single channel the
  design refuses. So the path is a declared fact, exactly as the identity is.

  **The cost is real and is the same cost the identity already carries: install
  and write can now disagree about *where* as well as about *who*.** Resolution
  is identical — **write wins**, because write holds the value that becomes the
  path, and an install that validated a different value validated something
  else. And the record **carries the sink as received**, so a disagreement is
  visible in the artifact rather than inferred from a file nobody can find.

  Two gaps become three payload-or-argv values reaching a filesystem location:
  `session_id`, `agent_id`, and now the sink root. The first two are validated
  as path components; the sink is a directory the user chose, so it is not
  charset-restricted — it is simply recorded, which is all that can honestly be
  done with a path somebody declared on purpose.

- **The declared identity becomes half a filename, so it is validated as a path
  component — measured, not recalled.** *Added 2026-08-17.* This is user-supplied
  configuration reaching a path, which is path traversal in a value the user
  controls, and it is not retention housekeeping.

  Probed on **Windows 10 Pro 19045, node v24.16.0**, with two controls — a
  known-good identity that must create a file, and a traversal that must escape,
  so neither a false accept nor a dead escape-detector can pass unnoticed:

  | Class | Measured |
  | --- | --- |
  | `../escape`, `..\escape` | **escaped to the parent directory** |
  | `a/b`, `a\b`, `a*b`, `a?b`, `a|b`, `a<b`, `a>b`, `a"b` | rejected by the OS |
  | `a:b` | wrote to an **NTFS alternate data stream** — no file, bytes gone |
  | `CON`, `NUL`, `PRN`, `AUX`, `COM0/1/9`, `LPT*`, bare and with `.jsonl` | **all created ordinary files** |
  | `ok.`, `ok ` | **created — the trailing dot and space are silently stripped** |

  **The device-name list is not the hazard here, and recalling it would have
  aimed the validation at the wrong thing.** Every reserved name created an
  ordinary file on this build; and in this design they are unreachable by
  construction anyway, because the filename is
  `<session_id>__<identity>.jsonl`, so the base name is never exactly a device
  name. Recorded with the build, because it is a property of this one.

  **The hazard the measurement did find is a collision, and it defeats the
  uniqueness check as specified.** Windows folds case and strips a trailing dot
  or space, so `foo`, `foo␠`, `foo.` and `Foo` are **four distinct declared
  identities and one file**. A uniqueness check comparing declared *strings*
  passes cleanly while the filesystem collapses them — which reproduces the twin
  writer this whole shape exists to make unrepresentable, through a check that
  looks correct.

  **So the requirement is two-part and the second half is the one that is easy to
  miss:**

  1. **The identity is restricted to a conservative charset** — ASCII
     alphanumerics and `-`, non-empty, bounded length. That rejects every
     row above by construction rather than by enumerating hazards, which is the
     closed-allowlist shape ADR-0005 §10 rule 4 uses for URLs and for the same
     reason: nobody writes down the traversal form they have not met.

     **`_` was in this set until 2026-08-19 and is not any more**, and the
     removal is the injectivity repair rather than tidying. With `_` admitted,
     the separator could be **formed at a boundary** by two legal components —
     `("sess", "abc_", "user")` and `("sess", "abc", "_user")` both rendered
     `sess__abc___user.jsonl`, and both were accepted (§2, round 3h). With `_`
     out, `__` cannot be formed from component content at all, the concatenation
     is **injective by construction**, and `ComponentRejection::ContainsSeparator`
     became unreachable and was deleted — the same move as `WriteStage::Flush`.
     **The two-component misattribution dies with it**: `sess_` + `X` and `sess`
     + `_X` both rendered `sess___X.jsonl`, which the reader parsed as
     `("sess", "_X")`.

     **The cost, and where it is heard.** `my_hook` must become `my-hook`, and
     **vibe does not normalise it** — silently substituting is inventing a
     plausible value, so vibe refuses and states what is permitted. An identity
     `vibe monitor install` writes is refused **at install, loudly, before
     anything is written**. A hand-written one (§7 permits those) is refused at
     **write** time inside the hook, whose stderr nobody reads, and the event is
     lost. So the repair **moves** that risk rather than removing it, which is
     the general property recorded below.
  2. **Uniqueness is checked on the normalised filename, not on the declared
     string** — case-folded, with trailing dots and spaces stripped. The
     filesystem's notion of *same file* is coarser than string equality, and the
     check must use the filesystem's, because that is the one that decides
     whether two writers share a file.

  **The charset's failure direction is non-delivery, and it reads as a guard.**
  *Added 2026-08-19.* A `session_id`, `agent_id` or identity outside the set is
  **refused, and the event is lost**. §7a frames the `session_id` case as *"loud
  loss beats silent invention"* — loud to the hook's **stderr**, which nobody
  reads, and silent to vibe. So it belongs on the list of non-delivery producers
  beside a broken hook and a killed process: **a charset that is too narrow does
  not corrupt anything, it deletes events.** n=5 on one build bounds nothing
  about the value space, and tightening the set (as the `_` removal does) moves
  that risk rather than removing it.

  Validated **at install and at write**, not at install alone: §7 permits
  hand-installed hooks, which install never sees.

- **`session_id` is the other half of the same filename, and this section was
  silent about it.** *Added 2026-08-18, from the writer.*

  The bullet above validates the **identity** as a path component because it is
  user-supplied configuration reaching a path. The filename is
  `<session_id>__<identity>.jsonl`, so `session_id` reaches a path by the
  identical route — and it arrives from **someone else's tool**, which is a
  weaker provenance than the user's own config rather than a stronger one. §3
  measured that every payload carries one on **2.1.233**; that is a property of a
  build, and a design that treats it as guaranteed has inherited a version.

  So it takes the same closed charset and the same bound. It is **not** the same
  value with a different name: the bounds differ, because the two components
  share one filename and Windows resolves a non-extended path against
  `MAX_PATH` = 260. Identity ≤ 48 and session ≤ 64 puts the longest filename at
  **120 characters**, leaving room for a sink directory. That arithmetic is the
  reason for the numbers, and raising either spends the headroom.

  **A payload whose `session_id` is missing, non-string, or unusable as a path
  component is refused, and the event is lost.** Stated as a loss rather than as
  handling, because it is one. The alternative is inventing a filename — a
  literal `unknown`, a hash, a fallback bucket — and an invented name can collide
  with a real session, which rebuilds the twin writer this whole shape exists to
  make unrepresentable. Loud loss beats silent invention; §7 already covers the
  consequence, since absence of events is not a state.

  *Recorded here rather than only in the module and its control, on the same
  argument the exit-code bullet above is recorded here: the decision site is
  where the next reader stands.*

#### Retention: vibe never deletes, and reports what is prunable

*Decided 2026-08-17.* One file per writer grows without bound, and this is the
only part of the design that would delete anything.

**Constraint 2's exemption exists in the text and does not survive the
mechanism.** The README reads *"No command deletes your files"* — scoped to the
user's files, which a sink vibe created is arguably not. But ADR-0001 §3 enforces
it by the **absence of `FileOp::Delete`**: *"a destructive command is not merely
discouraged, it is unrepresentable."* An absent enum variant has no scope. So any
vibe-side deletion must either add that variant — re-arming deletion at every op
site to serve one feature, converting a structural guarantee back into a
discipline — or write outside `plan`/`apply`, which is a second mutation path
with no dry run, no preconditions and none of ADR-0005 §10's containment.

**And it makes ADR-0005 §10 rule 5's TOCTOU residual materially worse**: today,
winning the parent-swap race gets an attacker a file **written**; with `Delete`
it gets one **removed**, which is not recoverable.

**So the automatic and the explicit forms cost the identical structural thing**,
and the difference between them — real as a question of consent — does not touch
it. An explicit prune looked cheap by analogy with ADR-0010 §3's permitted
`install`, and that precedent is about **writing**, which was already
representable.

**Decided: vibe computes and displays what is prunable, and the user deletes.**
No `FileOp::Delete`, no second write path, constraint 2 untouched in text and in
enforcement. It prints **paths, never a paste-ready command**, per ADR-0010 §9 —
which is also what the identity charset above makes safe.

**Stated without overclaiming, because the label is the trap:** *this does not
solve growth.* It is *"never delete"* plus a reporting tool, and **the
degradation is identical until someone acts**. All constraint 2 permits is making
the growth visible and actionable; saying more would be the label reaching one
step past what the mechanism does, which §5's genealogy records four times.

**The cost of not acting is measured rather than asserted.** A stateless read of
the whole sink, on this machine (Windows 10 Pro 19045, node v24.16.0), against
records of the measured size:

| files | enumerate | read + parse | on disk | records |
| --- | --- | --- | --- | --- |
| 100 | 0.2 ms | 71 ms | 2.7 MB | 3,925 |
| 1,000 | 0.9 ms | 629 ms | 27.2 MB | 39,257 |
| 7,300 — about a year at ten sessions a day | 6.2 ms | **4,679 ms** | 198.8 MB | 286,577 |

Linear, and enumeration is free — the cost is reading and parsing. **WARM CACHE,
recorded in capitals because it is the optimistic bound**: the files were written
immediately before being read, so a cold read is worse by an unmeasured amount.

**That number decides the ergonomics rather than describing them.** Constraint 4
sets this project's performance bar at *fifty repositories in well under two
seconds*; a monitor listing at one year is **more than twice that**, warm. At
1,000 files — roughly seven weeks — it is 629 ms and unremarkable. **So the
reporting half is not garnish, it is what keeps the feature usable**, and it must
surface before the cost is noticeable rather than on request. That is ADR-0010
§5's argument exactly: the state is shown by default because the failure is *"I
didn't notice"*.

**The fallback, named so it is taken deliberately if it is ever taken:** if the
ergonomics prove unacceptable in use, the answer is an explicit prune — and it is
adopted as *"we are adding `FileOp::Delete` and re-making ADR-0005 §10's
containment argument for it"*, never as *"we are adding a prune command"*.

**Prunability is derived from the records, never from remembered state.** The
reader is **stateless**: it walks the sink and reads everything, every time. A
read-watermark is a second artifact that must be kept equal to the files — the
shape ADR-0010 §3 rejected — and its staleness is silent in the dangerous
direction, since a watermark ahead of the truth **hides records**. So
*"unconsumed"* is not a state vibe holds, and the price of that is exactly the
read cost above.

It follows that **vibe cannot distinguish pruning a file whose records were read
from one whose were not**, and it must not pretend to.

**RETRACTED 2026-08-19: it cannot derive prunability from the artifact either.**
This paragraph used to read *"a file containing `SessionEnd` is a completed
session, and §4 measured that a killed agent writes neither `Stop` nor
`SessionEnd` — so a file lacking it is in-progress-or-dead and must never be
offered as prunable."* Both halves of that are still true and the conclusion does
not follow from them, because **`SessionEnd` is not terminal**: §2's round 3e
measured a resumed session emitting `SessionStart` *after* `SessionEnd` under one
`session_id`.

**The weaker replacement was checked before being built and failed too.** A third
state — *ended at least once, and reopenable* — carries no information, because
**`reopenable` is always true**: a session two hours old was resumed from a
different working directory, no `SessionEnd.reason` bounds it, and the reason set
is unenumerated. A variant whose predicate never varies distinguishes nothing.

**So prunability is not offered at all.** Not a weaker label, not file age.
Whether a file will receive more records is **not a function of the events in
it**, and constraint 5 says the field stays empty and flagged rather than
carrying a plausible value. The type is deleted rather than softened, which is
this document's usual move for a state that should not be representable.

**What must not be written in its place, recorded where the next reader will
stand:** *"a file untouched for N days is prunable"* is the same claim on a worse
basis. Age measures **when vibe last received an event**, which is the observable
§7 spends its length establishing means nothing on its own — a quiet agent, a
removed hook and a finished session all produce it. What could ground it is an
**explicit user action**, because that is a fact vibe was told rather than one it
inferred; nothing is built for that and §8 has not asked.

**The cost of having shipped the wrong label was bounded structurally**, not by
the display being unbuilt: no `FileOp::Delete` exists, so nothing vibe can do
could act on it. **The reason for the retraction is constraint 5** — the label
claimed what the tool does not know — and that reason survives §8 shipping a
display, where *"nothing renders it"* would not.

The read/unread distinction is preserved **by showing the user** — per file: the
session, the event count and the time span — not by vibe knowing, and not by
vibe recommending.

**And the residual is stated rather than dissolved:** a file that stops growing
is indistinguishable from a quiet agent. Neither transport ever fixed that. It is
§7's hazard, it is answered by the per-session wiring proof and by §5's liveness
check failing in different directions, and **the transport decision was never
capable of touching it** — which is the thing the first version of this section
got wrong.

### 7b. What install writes, where, and which events

*Decided 2026-08-19, after round 3's measurements. Recorded here rather than in
§8 because these were the three items §8 was waiting on, and a list of open
questions that still names a decided one is the failure §7a's own omission
records.*

#### One sink, installed at user level

**Decided: a single user-level hook writing to a single sink**, its path
resolved once at install time from `agents::default_store_path`'s precedent —
`ProjectDirs::data_dir()`, the data directory rather than the cache — and baked
into the config, per §7a.

**The reader was the deciding input.** `read_sink` takes one directory and no
caller merges. Three of its outputs do not survive N sinks:

- **`sequencing` is sink-wide.** Two sinks that are each `FullyOrdered` union to
  `PartlyUnordered`, because records in different sessions are unordered by
  construction. Merging listings after the fact reports a sequence where there
  is none — a plausible history, which is the one nobody investigates.
- **`identity_collisions` changes meaning.** Two identities colliding under
  `file_key` in *different* sinks do not share a file, so they are not the twin
  writer the check exists for — while remaining indistinguishable to §2's
  dedupe, which identifies a delivery by its source.
- **One unreadable sink among N has no representation.** `read_sink` returns an
  error for the whole read, deliberately distinct from an empty sink. Nothing
  can say *"three of four read"*, and a missing sink rendering as an empty one
  is §6's *absence of events is not a state*, one level up.

**None of that is a reason to change the reader, because N sinks were never
required.** One sink keeps `read_sink` as it is, keeps sequencing coherent over
the whole history, and lets `identity_collisions` keep the meaning it already
has. **It also collapses the staleness story**: one baked executable path and
one baked sink path, one blast radius, one health check — where per-project
install multiplies both by the number of projects and gives each its own way to
go stale.

#### The containment argument for `~/.claude/settings.json`

*Required because containment is re-established at every new call site rather
than inherited — ADR-0008 §6 — and this is a new one.*

ADR-0005 §10 rule 5 admits a write whose deepest existing ancestor canonicalizes
inside **a configured root or the plan's declared target directory**. The
user-level settings file is in neither, and widening the rule to admit "the home
directory" would trade a bounded invariant for an unbounded one.

**So the route is a closed variant, not a widened root, and it has ONE member.**
*Corrected 2026-08-19: the first draft made it two-valued, user or project.
Project-level install was closed by the decision above, so no command could
produce that variant — a representable state with no producer, and the moment
anything reached it all three of the reader's problems come back.* The technique
is ADR-0001 §3's missing `FileOp::Delete` and ADR-0005 §10 rule 1's constructed
argv: **the op names no path, and carries no choice either.** `apply` resolves
the one target itself. A plan cannot express a write to an arbitrary place
outside a root because there is no field to put one in, and cannot express a
project-level settings write because there is no variant for it.

§9's obligation is the set, not the member: the control asserts the variant set
has **exactly one** member, so adding a second turns it red — which is what
makes re-opening project-level install a decision rather than a diff.

**What makes that safe here is a property of this write, not a general
permission:** the target path has **no component derived from data**. Not from a
payload, not from a scan, not from a project name, not from anything a user
typed. It is the home directory plus two fixed literals. The traversal class
rule 5 exists to stop is a path *assembled* from values; nothing is assembled
here, so the hazard does not arise rather than being checked for. Compare the
sink filename, which has three payload-or-argv components reaching a path and is
charset-validated at both ends precisely because it does.

**Rule 6 is unaffected** — the path is not under `.git/` and cannot be made to
be. **Rule 5 still runs**, against the resolved path, because a check skipped on
the ground that it cannot fail is a check that stops running when the ground
moves.

**The read side needs no route and must not borrow this one.** §7's contract
read and §7a's uniqueness check both read two settings files. ADR-0001 §3
governs *mutation*; reads are not `FileOp`s and need nothing. Stated so that a
later change does not widen the write variant to serve a reader.

**What this costs, stated rather than absorbed:** the invariant stops being
*"vibe never writes outside a configured root"* and becomes *"…except one named
path, reachable only through one variant, only from `vibe monitor install`"*.
That exception is enumerable, and a control that enumerates it is the only thing
keeping it from growing a second member quietly.

#### Records arrive from projects the registry has never seen

**A user-level hook fires everywhere**, including in directories that are not
registered projects. That is new: every other thing vibe reports is about a
project it was told about.

**Such records are shown as unregistered.** Not dropped — the events are real
and the session is named in every payload. Not attributed to a nearest match —
that is inventing the fact that decides which project a record belongs to, which
is constraint 5 pointed at attribution. It is `unattributed`'s neighbour (§7a):
a fact vibe does not have, named as missing, with everything not depending on it
still usable. The payload carries `cwd`, so *where* is known; what is unknown is
which registry entry, if any, it corresponds to.

#### The editor: `serde_json` with `preserve_order`, and `to_string_pretty`

**Decided.** Measured inputs: the file is strict JSON so there are no comments
to lose (§7); `preserve_order` adds **no crate**, because `indexmap` is already
in the graph via `toml_edit` and `serde_json`'s only reverse dependents are this
workspace's own two crates; and the formatting cost is the table in §7 — zero
lines on a 2-space file, 44 of 49 on a 4-space one.

**The formatting risk is an output, not a blocker**, because `FileOp::UpdateFile`
already carries `before` and `after` so `--dry-run` renders the real diff before
anything is written (ADR-0001 §3). A cost the user sees in advance is not the
same kind of cost as one they discover afterwards.

**The formatting is sniffed rather than imposed.** *Decided 2026-08-19, and it
turns the cost table into one row that matters.* `PrettyFormatter::with_indent`
takes the indent as bytes, so the editor reads the existing indent off the first
indented line and reuses it, and does the same for the line ending. Measured on
the re-install fixture, the same 49 lines formatted five ways:

| formatted as | rewritten, naive | rewritten, sniffing |
| --- | --- | --- |
| 2-space LF | 0 | **0** |
| 4-space LF | 44 | **0** |
| tab LF | 46 | **0** |
| 2-space **CRLF** | **48** | **0** |
| 4-space CRLF | 48 | **0** |

**The CRLF row is the one that was missing and it is the one from this
project's own platform.** `to_string_pretty` emits `\n`; a CRLF
`settings.json` on Windows is rewritten almost whole. Every row is
byte-identical after sniffing.

**The CRLF rewrite is a global newline replace, and that is safe for a reason
worth asserting rather than assuming:** JSON escapes a newline inside a string
as the two characters `\` and `n`, so a literal newline **byte** can only ever
be formatting. Measured both ways — a string containing an escaped newline
survives untouched, and the formatting newlines really were rewritten, so the
check is not satisfied by a rewrite that did nothing.

**The residuals are declared rather than discovered**, since sniffing is a
heuristic and a heuristic that is silent about its misses is the thing this
project keeps refusing:

- **Nothing to sniff** — a minified file, or one with no indented line. Default
  to two spaces. Measured: the sniffer returns `"  "` for a minified file.
- **Mixed indentation** — the sniffer takes the **first** indented line, which
  may not represent the file. Measured: a 2-space file with one tab-indented
  line sniffs as tab, and would then be rewritten whole.
- **Mixed line endings** — the newline sniff takes the first occurrence in
  exactly the same way and carries exactly the same residual. A file with both
  `
` and `
` in it is normalised to whichever appears first, and the
  other lines are rewritten. Declared beside the indentation residual because
  the two are one heuristic applied twice, and declaring only one of them would
  read as if the other had been checked.

Both land in `--dry-run`'s diff before anything is written, which is what
constraint 2 is for and why neither is a blocker.

**The prerequisite was a control repair, not a follow-up, and it is done.**
`the_payload_lands_byte_identical_including_key_order` had **two** producers of
its observable — key order and number formatting — and `preserve_order` removes
the first. It would have stayed green on the survivor while its name, its doc
comment and its premise all described something it no longer exercised. The
branches were counted again before the flag moved: under `preserve_order` a
naive round trip still changes **number formatting**, **insignificant
whitespace** and **a duplicate key** — and no longer changes key order, a large
integer, or an escaped non-ASCII character. One control per producer now, each
with its own premise assertion, plus
`serde_json_in_this_build_preserves_key_order`, which fails if the feature is
ever dropped — because nothing else would, and the damage would be vibe
re-sorting the keys of a file it does not own. That control was **red before the
flag and green after**, which is what establishes it tests the flag rather than
the crate.

#### How the write lands, and the `--dry-run` that precedes it

*Added 2026-08-19. The editor produced text for four rounds and nobody asked how
the text reached the disk — which is where the only genuinely destructive act in
this tool lives.*

**The route is `FileOp::UpdateFile` through `Registry::apply`**, so nothing new
mutates the filesystem (ADR-0001 §3) and `--dry-run` is `plan_*()` plus render.
`UpdateFile` carries `before` and `after`, so the diff a user sees is the real
one — which is what makes the sniffing residuals in this section an **output**
rather than a blocker: mixed indentation and mixed line endings both surface as
a diff before anything is written.

**`apply` replaces atomically now, and it did not before.** `std::fs::write`
truncates before writing, so the target was observably **zero bytes** part way
through — measured, with a paired control catching it (§2, round 3f). The write
goes to a temporary file **beside the target** and renames over. Same volume, so
the rename is a rename rather than a copy-plus-delete.

**What it does not promise:** durability. No `fsync`. A power failure can lose
the new contents; it cannot leave the file empty or half written. Stated because
*"atomic"* is a word that invites the stronger reading.

**A refusal happens before anything opens for writing.** The parse is the first
step and an unparseable file never reaches the write, which matters because the
syntactic case is the likely one: users edit this file by hand, and §7 measured
the loader refusing comments and trailing commas. Controlled against the **bytes
on disk** rather than against a `Result` — *"parse returned an error"* and
*"nothing was written"* are different claims and only the second is the one that
reaches somebody's config.

#### Events: the lifecycle five, and the rest deferred with a reason

**Installed: `SessionStart`, `SessionEnd`, `SubagentStart`, `SubagentStop`,
`Stop`.** Lifecycle plus a per-turn heartbeat, and every one is in the measured
ten (§2).

**Deferred, with the reason recorded rather than left as an omission:**
`PreToolUse`, `PostToolUse`, `PostToolBatch`, `MessageDisplay`,
`UserPromptSubmit` are **activity-rate** events and nobody has measured write
volume for a real session. The 24 cross-identity overlaps in §7a's round came
from six tool calls; a working session is not six.

**Nothing from the twenty-one is installed**, including `TaskCreated` and
`TaskCompleted` — which **are** among the 31 accepted names and did **not** fire
(§2). The original product question about them is therefore unchanged rather
than answered: they sit in the set that neither fired nor was shown to be
correctly registered.

#### Install writes every field it depends on, explicitly

**Decided 2026-08-19, corrected the same day once the field types were read.**
The emitted hook carries, in one group of its own:

```json
{ "matcher": "*",
  "hooks": [ { "type": "command", "command": "<vibe>", "args": [ … ],
               "once": false, "async": false, "asyncRewake": false,
               "timeout": 5 } ] }
```

**This is not verbosity, it deletes a dependency.** Otherwise install's
correctness rests on measured defaults staying put across upstream releases with
nothing watching them, which is ADR-0005 §10 rule 4a's untriggered channel
pointed straight at delivery.

**Measured end to end before being written down:** that exact group is accepted
by the loader with no complaint and **fires on all five lifecycle events**, 1:1
against a bare control in the same file (§2, round 3d). `asyncRewake: false` is
accepted alongside `async: false`, which was the condition attached to writing
it, so it is written rather than registered.

**`matcher: "*"` was blocked and is now cleared, on the right class.** The first
measurement was on a **tool** event, where a matcher filters tool names; on
`SessionStart` it filters something else or nothing at all. Installing against
that would have been a control proving one hazard class and shipping against
another — and if `"*"` had not matched, the hook would never fire, which is
exactly §6's suppressed-hook observable written into the config by install
itself. Re-measured on the lifecycle five inside install's own group: it fires.

**`shell` is NOT written, and the reason is a correction rather than an
omission.** `shell` is a string enum of `"bash"` and `"powershell"` — **there is
no value meaning *no shell***, and `shell: false` is refused by the loader per
hook. What closes it is `args`: with the exec form present, `shell` written and
`shell` omitted produce byte-identical argv (§2, round 3b). **So the dependency
install carries here is on `args`, which it writes**, and it is recorded as that
rather than as a dependency on `shell`'s default.

**`if` remains the one residual omission-dependency.** No value meaning *do not
suppress* exists — `if: "*"` is accepted by the loader and fires nothing, which
is the silent failure `shell: false`'s loud one is not. Omission is permissive
today, so the failure direction is **suppression**, and §6 forbids rendering
that as an idle agent. It is also the one exclusion from §9's prohibition on
testing upstream defaults.

#### The `timeout` value comes from a measurement, and the rule outlives the number

**Decided 2026-08-19.** The dependent that blocked this — the fear that a torn
line would darken a whole sink read — is resolved: it does not, and the control
saying so runs on three platforms.

**Measured input.** `a_cold_hook_invocation_is_measured_and_reported_per_platform`
times the whole invocation as Claude Code experiences it — spawn, `clap` parse,
stdin read, append, exit — over ten cold runs, in the ordinary test job, so it
reports on **all three platforms** rather than on the one every other
measurement here was taken on. On `windows/x86_64` in the test profile: min
**11.7 ms**, median **22.8 ms**, max **37.5 ms**.

**The rule has TWO TERMS with different subjects, and the first draft
conflated them.** *Corrected 2026-08-19.* Cold start is bimodal (§2, round 3e),
so *"the maximum"* was whichever mode the run sampled:

- **A multiplier over the STEADY-STATE maximum** — catches gradual regression in
  what the hook does.
- **A floor over the COLD-MODE maximum, with a stated margin** — catches the
  paging case, which is where `SessionStart` lands.

**The floor is currently inherited rather than derived, and that is recorded as
a debt.** 5 s covers the 1.20–1.27 s cold measurements at about 4×, but the 5
was chosen before those numbers existed. The multiplier has never bound, so the
floor has carried the whole load without anybody deriving it for that job. It is
derived from the cold distribution once CI has more than two samples on more
than one platform, and **the margin is recorded as chosen rather than inherited**
when it is. If it lands back on 5 s, good — but it will have been derived.

**So the rule as it stands:** `timeout` is **100× the largest steady-state
maximum across the three platforms, rounded up to a whole second, with a floor
of 5 s** — the floor provisional per the paragraph above. At 37.5 ms that is 3.75 s,
so the floor decides and the value is **`timeout: 5`**. It is not a performance
budget: almost all of the measured time is process startup, and the multiple is
there to survive a cold, loaded, virus-scanned machine that no benchmark here
resembles.

**Three things about that number, all of which the first draft left implicit.**
*Added 2026-08-19.*

- **Which binary.** The measurement is the **test profile**, which is debug; the
  installed hook invokes the **release** binary. Debug bounds release from above,
  so the derived value is conservative in the safe direction — but it is *a*
  binary rather than *the* one, and which binary was invoked is the question this
  document has had to ask at every round.
- **It is PROVISIONAL, not derived.** The rule names the largest maximum across
  three platforms. One has reported. `timeout: 5` currently rests on
  `win32-x64` alone and stays provisional until CI reports the other two.
- **The multiplier branch has never bound.** 37.5 ms × 100 is 3.75 s, under the
  floor, so the floor has decided every time. A two-branch rule with one branch
  never exercised carries an untested claim — that the multiplier does anything —
  and it starts binding the moment a platform reports over 50 ms.

**And the derivation is a control rather than a transcription.**
`the_installed_timeout_is_what_the_rule_derives` recomputes the requirement from
the measurement on whichever platform is running and asserts the installed
constant clears it. Each platform checks its own half; the three together check
the rule. Without it the derivation ran once and the number aged silently, which
is the failure mode this document keeps finding in prose.

**One thing the measurement itself turned up: cold start is bimodal.** The first
invocation after a build measured **1.27 s** against a steady state of about
22 ms — the operating system loading a 5.6 MB binary written seconds earlier. A
rule multiplying *the maximum* therefore multiplies whichever mode the run
sampled, and a single cold sample would derive 127 s. The controls take **one
untimed warm-up** and time the steady state, printing the discarded number
rather than hiding it. **Whether the rule should name the cold mode instead is
open**: a hook is invoked many times per session and only the first pays it, but
the first is also the one that fires at `SessionStart`, which is §7's wiring
proof.

**The cost runs in both directions and both are stated.** The hook is
**blocking** — measured, since `async` omitted or false makes the session wait
(§2, round 3b) — so `timeout` is the ceiling on how long **one wedged hook can
stall the agent**, per event. Install writes five events, and `Stop` fires per
turn, so a wedged hook costs up to `timeout` **per occurrence** rather than once
per session. Larger is safer against losing a record; smaller is safer against
stalling somebody's work. Five seconds against a 37.5 ms measurement is a
factor of 133, and it is chosen deliberately toward not losing the record,
because §7a already decided that an observer that can stop the subject is not
one — the *stall* is the failure this trade must not make routine, and a hook
that hangs at all is already a fault.

#### The hazard is a killed hook, not the `async` field

*Corrected 2026-08-19. An earlier draft forbade `async: true` and justified the
prohibition with the observable — process killed, start on disk, end never
written. **That observable has more than one producer**, so a prohibition
written over the field leaves install free to reproduce the hazard by other
means without violating anything recorded.*

**Counted, the producers are three:**

1. **`async: true` at session end.** Measured (§2, round 3b): the hook returned
   to the background, the session ended, and the end line was never written.
2. **A `timeout` shorter than the hook takes.** Measured: `timeout: 2` against a
   5 s hook leaves start and no end.
3. **Claude Code exiting, the machine sleeping, or the process being killed by
   anything else.** **Not eliminable by any config vibe writes**, and therefore
   the reason the hazard has to be stated over the outcome rather than over the
   fields.

**So the rule is: install must not increase the exposure of a hook to being
killed mid-flight, and the two producers it controls are named.** `async` is
never written true; `timeout` is written explicitly, at a value chosen against
round 3c's bound rather than against the unlocated default. The third producer
is why §7's per-session wiring proof and §5's liveness check both still exist —
they fail in different directions, and this is a case only they cover.

**And round 3c bounds what the third producer costs**, which is what makes the
first two worth constraining rather than despairing of: on the platform
measured, a kill leaves nothing or a whole record, never a torn one; and a torn
record, if one ever occurs, costs its own file's tail and nothing else.

### 8. Open, and not mine to decide

*Revised 2026-08-17. The previous list had two items and read as complete; it was
missing transport, which is now §7a. What follows is the state after round 2.*

*Revised again 2026-08-19. **Three items left this list by being decided, not by
being dropped**: install scope, the settings editor, and which events to install
are in §7b. They were never numbered here — they lived in a handoff note — and
that is recorded because a list of open questions is only useful if the things
that leave it say where they went.*

1. **~~Whether `PermissionRequest` and `Notification` are used~~ — answered on one
   path, open on the other.** They do **not** fire headlessly through the MCP
   permission route, with the names registered and the mechanism demonstrably run
   (§5). Whether the interactive TUI path fires them is **unmeasured and not
   measurable here**, for want of a constructible pty. What closes it is a fixture
   with a real terminal, not another headless run.
2. **The display shape and the state names.** Unchanged as a question and
   **harder as a problem**, now for two reasons: §5's retraction means the facts
   available are fewer than round 1 recorded, so a display cannot lean on *"a
   tool is running"*; and §7a's transport makes history a **partial order**, so
   the display inherits an *unordered with respect to each other* state it cannot
   render as a sequence. §6 constrains this and does not settle it.
3. **~~Transport~~ — decided: a file, §7a.** `http` was chosen first on a real
   and unique property and **reversed** when costing showed that property to be
   information about the receiver rather than about the subject. The reversal is
   recorded in full, with the falsification list that licenses it, because the
   next reader overturns it by finding a candidate that list missed.
4. **New: whether `PostToolBatch` is the closer `PostToolUse` is not.** Two
   observations, single-tool batches only; parallel and nested semantics
   unmeasured (§5). This is the first thing to measure when the feature is picked
   up, and it is blocked on measurement rather than on preference.

   **Half-answered on 2.1.234, and the half that moved is the fixture.** *Added
   2026-08-19.* Multi-tool batches exist: `tool_calls` held **two** entries in
   one run and **six** in another, both from a subagent issuing parallel `Read`
   calls, so *"single-tool batches only"* was a property of the fixtures rather
   than of the event. What is still open is the part the question was actually
   about — **whether the batch closes anything `PostToolUse` leaves open** — and
   that is unchanged: every call in both batches also produced its own
   `PreToolUse`/`PostToolUse` pair, so nothing observed requires the batch to be
   a closer. Nested semantics remain unmeasured.

### 9. Negative-control obligations for whatever gets built

- **The third state needs a paired control**, per ADR-0002 §7's rule against
  one-sided ones: an agent killed mid-tool must render as the third state, **and
  a live agent with the same last event must render as working**. A build that
  reports the third state unconditionally satisfies the first half perfectly.
- **The dedupe needs a control that both files are declaring hooks**, or it tests
  nothing — the union in §2 is the precondition, and a fixture with hooks in one
  file only would pass while the defect ships.
- **The wiring proof needs its own paired control**, because it is the one thing
  everything else rests on: a session with hooks installed must render as
  *observed*, **and the same session with the hook config removed must render as
  unknown rather than idle**. A build that renders unknown only when it has no
  events at all satisfies neither half meaningfully, and the failure is silent by
  construction.
- **Any further measurement of Claude Code needs a channel control first** —
  ADR-0002 §7's channel rule, whose base rate came from this feature's own
  measurement round, where six of six discrepancies were the instrument.
  **Round 2 honoured this**: `probe.js` was rebuilt with its known line-by-line
  blindness fixed and demonstrated in both directions — whole-text matching finds
  a multi-line target, the line-by-line sabotage does not, and the control is
  identical in both, which is exactly why the original blindness survived — plus
  a dead-control run proving a zero cannot be reported without a live control.

- **The `(pid, start_time)` liveness check needs a paired control on the
  `OpenProcess` failure, not on the handle.** *Added 2026-08-17.* NULL is two
  outcomes sharing one observable (§5), so: **PID 4 must render as
  *unavailable*, and a nonexistent PID must render as *gone*, and the two must
  render differently.** A build that maps every NULL to *gone* satisfies the
  second half perfectly while reporting a live agent as stopped. The fixture's
  dependence on running **non-elevated**, and on PID 4 being the protected
  `System` process, belongs in a comment beside it — those are facts about the
  environment, and when one changes the test stops testing without going red.

- **FORBIDDEN: do not write a control asserting that an open `tool_use_id`
  renders as working.** *Added 2026-08-17.* It is the control §5's retracted
  sentence would have justified, it will pass, and **passing is the defect** — an
  open id also belongs to a denied tool in a session that has stopped (§5). A
  build that renders an open id as *working* is wrong on 2.1.233, and this
  control would certify it as right.

  **The general rule is in ADR-0002 §7, not here** — *a retraction removes a
  control's subject, and nothing inherits the removal; record what must not be
  built beside what must.* It is filed there for the reason this project has now
  paid for twice: the next person writing a control will be reading the rules,
  and will not open the monitoring ADR.

- **The one-writer-per-file property needs a control, and "several simultaneous
  writers" is not it.** *Rewritten 2026-08-17, once §7a settled on D. The first
  version of this bullet asked for simultaneous writers appending to one sink and
  per-record parseability, and it would have **passed while proving nothing**: the
  failure it aimed at is size-dependent, not concurrency-dependent, and small
  records from several writers reproduce nothing.*

  Two controls, because the design has two claims:

  **a. Distinct paths, paired, and on three axes rather than two.** *Rewritten
  again 2026-08-17, because the two-axis version had the defect it was written to
  prevent.* It required two sessions and §2's two settings sources — and used
  **one hook per source**, so it would have passed against a build keying on the
  settings file, which is the defect that was actually shipped. The axes are:

  - two **sessions**, one hook each;
  - two **settings sources** within one session;
  - **two hooks declared in the same settings file** for the same event — the
    axis the earlier fixture was silent on, and the one that separates *"one file
    per declared identity"* from *"one file per settings file"*.

  Every combination must produce a distinct file. Sabotage by collapsing the
  naming to a constant and observing the collision; sabotage a second time by
  keying on the settings source and observing **only the third axis go red**,
  which is what establishes that the third axis is doing work the other two
  cannot.

  **b. A cut-mid-record tail.** Write a file whose final record is truncated
  part-way and assert the reader **reports a partial tail** — not dropping it
  silently, and not parsing a prefix that happens to be valid. Sabotage by
  deleting the tail check and observing the truncated record either vanish or be
  accepted as complete. This is constructible by hand precisely because D bounds
  the hazard positionally; it needs no race, which is the property that made the
  rejected shared-file shape inadmissible.

  **Note what is deliberately not controlled: append atomicity.** Under D nothing
  concurrent happens, so there is no atomicity property to test. A control
  asserting it would be testing an OS guarantee this design no longer depends on
  — and it would go green on the one platform it could run on, which is how the
  dependency got accepted in the first place.

  **c. Uniqueness is refused, paired.** A config declaring the same identity
  twice must be **refused by `vibe monitor install` and reported by the contract
  read**, and the same config with distinct identities must be accepted and
  produce two files. Sabotage by deleting the uniqueness check and observing the
  duplicate accepted. The fixture must reach the check rather than failing
  earlier — a config that is malformed for some other reason proves nothing about
  it.

  **d. The identity is validated as a path component, paired.** A traversal
  identity (`../escape`), a separator, and a `:` must each be **refused at
  install and at write**; a valid identity must be accepted and produce a file.
  Sabotage by removing the charset check and asserting the traversal case
  **writes outside the sink** — which is the assertion that makes this about
  containment rather than about a rejected string.

  **`../escape` is the wrong fixture and it passes, which is worse than the
  prohibition above.** *Added 2026-08-18, measured while building the writer.*
  There the rule says do not write a control; here the wrong control is the one
  written **first**, because `../escape` is the obvious shape of path traversal
  and needs no telling.

  It does not escape. The identity is **flanked** — `<session>__` in front,
  `.jsonl` behind — so the component is the literal directory name `sess__..`,
  which is not a parent reference. The write fails with `NotFound`, and a
  fixture asserting only that the write failed **reads as proof of containment
  while certifying a build with no validation at all.** The escaping form puts
  the `..` in a **middle** segment, where nothing flanks it:
  `x/../../escape` wrote 193 bytes into the sink's parent on Windows 10 Pro
  19045 with the charset check removed.

  **The tell is that the flanking is invisible in the result** — a refused
  traversal and a traversal that was never one produce the identical failed
  write. So the assertion is on **where the bytes landed**, enumerating the
  sink's parent before and after, never on whether the call errored.

  **And the reachability premise is itself a measurement, which inverted the
  platform asymmetry §5 predicted.** The escape is reachable on **Windows**,
  which canonicalises `..` lexically; it is **not** on Linux (WSL2 6.18.33.2,
  two independent instruments) or macOS (measured by the assertion, green on
  `macos-latest`), which resolve `..` against real directories so the
  nonexistent `sess__x` stops it. Windows holds the reachable fixture and Unix
  does not — the inverse of ADR-0010 §10. On Unix the control guards a hazard
  unreachable through this filename scheme, and that is a **declared platform
  limit, not coverage**.

  **The general rule is in ADR-0002 §7, not here**, beside the retraction
  prohibition — because the next person writing a control will be reading the
  rules and will not open this document.

  **e. Uniqueness is checked on the normalised filename, not the declared
  string.** Two identities differing only by case, or by a trailing dot or space,
  must be **refused as duplicates** — measured on Windows 10 Pro 19045 to be four
  distinct strings resolving to one file. Paired: two genuinely distinct
  identities must be accepted. Sabotage by comparing raw strings and observing
  the collision accepted, which is the twin writer arriving through a check that
  reads as correct.

  **f. RETRACTED 2026-08-19 — prunability is not derivable from event
  content.** This required a paired control on *"a file containing `SessionEnd`
  is offered as prunable"*. Two controls were built, both passed throughout, and
  both were pinning a claim the payload does not support: `SessionEnd` is not
  terminal, and *reopenable* is always true. The obligation is withdrawn with its
  subject, per ADR-0002 §7's rule that a retraction removes a control's subject
  and nothing inherits the removal.

  **Nothing replaces it.** A control asserting that no prunability is offered
  would be asserting the absence of a type, which the compiler already does. And
  see `monitor::sink`'s docs for what must **not** be built here — file-age
  prunability — since a later reader will propose it.

- **DECLARED GAP: two of the writer's four failure stages have no control.**
  *Added 2026-08-18.* `CreateSink` and `OpenFile` are controlled and paired — a
  file planted where the sink must go, a directory planted at the record path.
  **`Append` and `Flush` are not**, and cannot be induced from this machine
  deterministically: they need a full volume, which is not constructible in a
  temporary directory, or a process killed mid-write, which is a race — and
  ADR-0002 §7 rejects a control whose firing depends on winning one, because it
  can stop proving anything without ever failing and it arrives as a green.

  **The cost was larger than a missing control and is now narrowed.** *Amended
  2026-08-18.* `torn_bytes` is computed only on those two arms, so the field had
  never held a measured value in **any** test — unexecuted code that would read
  as a fact the first time it appeared in a record.

  The **body** is now covered: the computation is factored out of the
  unreachable branch and exercised over its inputs, including against a real
  truncated file. That pass found the computation **wrong in the reassuring
  direction** — a failed `stat` folded into `0`, reporting *"nothing was torn"*
  when nothing was known — which is the `Process.StartTime` class one layer
  down. It returns `None` now, and unknown propagates.

  **The dispatch stays uncovered**, which is a narrower gap than before and is
  still a gap: what is untested is *which branch calls it*, not *what it
  computes*. Recorded as a gap rather
  than left to read as coverage, since four variants with two controls look
  uniform from outside.

- **AMENDED: the gap is one arm, because `Flush` was deleted rather than
  covered.** *2026-08-19.* `File::flush` costs ~0 ns against `sync_all`'s ~90 µs
  — paired against a call known to syscall — and there is no buffered writer in
  the path, so the branch could not be taken. An unreachable variant is a
  representable invalid state, and the rule here is to make those
  unrepresentable rather than filter them. The variant, its arm and the `flush`
  call are gone; the residue was swept at the copy sites as well as the decision
  site, including `WriteOutcome::Written`'s *"on disk and flushed"*.

  **The enum survives and the gap narrows rather than closing**: three variants
  remain, two controlled, and `Append` is the one that needs a full volume.

  **Both properties rest on the writer being a bare `File`, and one commit would
  take both.** A `BufWriter` makes `flush` live *and* turns `write_all` into a
  loop. Guarded by two controls rather than by a paragraph:
  `the_write_path_has_no_buffered_writer` reads the module's own source — the
  technique `control_inventory.rs` already uses, for the same reason — and
  `a_written_record_is_on_disk_before_append_returns` catches buffering by its
  effect, including through a type not called `BufWriter`. Two instruments,
  because the first is a string match and string matches are as literal as they
  look.

- **DEFECT, recorded rather than repaired: `SessionEnd` no longer licenses
  `Prunable`.** *Added 2026-08-19.* §2's round 3e measured a resumed session
  emitting `SessionStart` **after** `SessionEnd` under one `session_id`, so a
  file containing `SessionEnd` may belong to a session that is still live.
  Control (f) below derives prunability from exactly that, which means it
  currently pins a claim the payload no longer supports.

  **Nothing renders it** — §8 leaves the display open — so the cost is bounded
  and the repair is not urgent. It is left as a decision because **what a label
  claims is a product question**, and the honest shape is this document's usual
  one: a third state, *ended at least once and reopenable*, never borrowing the
  appearance of *finished*. Recorded here so the next person to touch (f) meets
  it, rather than in a note that gets deleted.

- **The inventory gate's numerator has come loose from the number of controls,
  and both are printed now.** *Added 2026-08-19.* ADR-0008 §9's trigger counts
  integration-test targets gated on a `VIBE_REQUIRE_*` variable, and that marker
  turns a **missing external tool** into a failure instead of a skip. Two rounds
  running, real controls landed and the gated count did not move — **because
  none of them needs `git` or `gh`, so none has a skip path to close.** Gating
  them would be wearing the marker rather than using it.

  So *"is a reviewer still holding the whole argument?"* is no longer answered by
  the number the gate watches. **Changing the trigger's definition is not this
  file's to do**, but the divergence being invisible is: the total is now
  derived and printed beside the gated count in the same invocation, so a round
  that adds five controls and moves the gate by zero says so in CI rather than in
  a report. At the time of writing: **252 `#[test]` items across 24 targets, 5 of
  them `VIBE_REQUIRE_`-gated, trigger at 7.**

- **The cold-start measurement is a control, and it is the one number in this
  document that comes from all three platforms.**
  `a_cold_hook_invocation_is_measured_and_reported_per_platform` times the real
  binary over ten cold invocations and prints min, median and max per platform,
  because §7b's `timeout` is a stated multiple of that maximum and a multiple of
  a single-platform number would inherit §9's limit.

  **Its assertion is a tripwire, not a budget**, and the distinction is why it
  is admissible at all: a tight timing assertion on a shared runner goes red for
  reasons unrelated to this code, which is the flake ADR-0002 §7 rejects because
  it trains people to ignore a control. The ceiling is ten seconds against a
  measured maximum of 37.5 ms — it fires only if the hook has started waiting on
  something it must not. **Paired**: the timed runs must also have written what
  they were timed for, or the measurement is of a hook that exited early.

- **Ordering needs a control that the reader refuses rather than guesses.** Two
  records with no ordering relation between them in the payload — different turns,
  no shared `tool_use_id`, no `index` — and stamps that are equal or inverted must
  render as **unordered**, not as a sequence. Paired: the same two records with a
  payload relation present must render **ordered**. A build that always emits a
  sequence satisfies the second half perfectly, and the failure is a plausible
  history, which is the one nobody investigates.

- **The contract version needs a control on the delivery properties, not only on
  the payload.** `timeout`, `async` and `asyncRewake` each change whether and
  when a record arrives (§2, §7a). A control that pins only the payload shape
  passes against a hook whose `async` variant delivers nothing at session end,
  which is silent non-delivery arriving through the mechanism installed to
  prevent it.

  **And the set is seven, not three.** *Amended 2026-08-19, see §7a.* `once`,
  `if`, `matcher` and `shell` belong to the same class and were not named. A
  control that covers three of seven and says so is honest; one that covers three
  and reads as covering the class is the quantifier failure this document keeps
  finding, so **whatever is built states which properties it pins and which it
  does not.**

- **DECLARED LIMIT: every measurement of Claude Code in this document is
  single-platform.** *Added 2026-08-19, and stated because the register was
  reading as if these were properties of the tool.* Rounds 1, 2, 3, 3b and 3c
  were all taken on **`win32-x64`, one binary, one machine**. Constraint 3 makes
  Linux, macOS and Windows first-class, and three of the findings are plausibly
  platform-dependent rather than merely unverified elsewhere:

  - **`shell`.** The default selects bash *on a Windows machine that has Git
    Bash*, and the schema's prose says powershell without it and nothing at all
    about `sh` versus `zsh` on Unix. **This is not a live risk to install and
    the wording used to imply it was.** *Reworded 2026-08-19.* `shell` has no
    value meaning *no shell*, so install closes it by writing `args` rather than
    by writing `shell` (§7b), and the exec form was measured to make `shell`
    inert. The correct label is **finding not carried across platforms; install
    does not depend on it.** What install *does* depend on is `args` behaving as
    the exec form everywhere, and that is the thing to re-measure on the other
    two.
  - **The intra-agent serialisation reading.** It rests on how this build
    spawns and waits on hook processes, which is a per-platform code path.
  - **The 15 ms concurrent-hook figure**, and every other latency here.
  - **Round 3c's kill result.** NTFS with an append handle is not ext4 or APFS,
    and `write_all` looping is exactly where the platforms could differ.

  **What is mechanized, and it is the half that has a dependent.** The reader's
  behaviour on damaged files — the thing a kill's cost actually reaches — is
  Rust, runs in the ordinary test job, and therefore runs on all three platforms
  in CI: `one_damaged_file_does_not_cost_the_sink_its_other_records`, paired and
  sabotage-checked. So *"a torn record costs its own file's tail and nothing
  else"* is three-platform even though *"a kill produces no torn record"* is
  one.

  **What is not mechanized, and why not rather than merely not yet.** Everything
  needing a real Claude Code binary and a live session cannot run in CI: there
  is no such binary on a runner, and a job that spent a model turn per push
  would be measuring somebody else's service on this project's schedule. The
  instruments are committed so the re-measurement is cheap on each of the three
  platforms when someone is standing on one; nothing runs them automatically,
  and §9's on-demand bullet says so.

- **The settings-target control asserts the set has exactly ONE member.**
  *Amended 2026-08-19.* §7b's route was two-valued in its first draft — user or
  project — while project-level install had already been closed, so a
  representable state existed that no command could produce. A control asserting
  *"this write lands at the user settings file"* passes just as happily with a
  second variant sitting beside it. The assertion is therefore on the cardinality
  of the variant set, so re-opening project-level install is a decision that
  turns something red rather than a diff that does not.

- **The prohibition on testing omission defaults is SCOPED, not blanket.**
  *Amended 2026-08-19.* For the properties §7b writes explicitly — `shell`,
  `async`, `once`, `asyncRewake`, `matcher`, `timeout` — a control asserting the
  build's default would report a fact about Claude Code as a defect here, and
  install no longer depends on any of them. **`if` is excluded**, and it is the
  only exclusion: it has no written value meaning *do not suppress*, so install
  must omit it and does depend on the default. A default that changed would
  silently stop delivery and **nothing else in this repository would catch it**,
  which is precisely the case the prohibition would otherwise close. Whatever is
  built for `if` states that it is measuring somebody else's build, so a red
  reads as *upstream moved* rather than as *we broke something*.

- **Records from an unregistered project need a paired control.** *Added
  2026-08-19 with §7b.* A user-level hook fires everywhere, so: a session in a
  directory the registry has never seen must render as **unregistered**, **and a
  session in a registered project must render as that project**. A build that
  labels everything unregistered satisfies the first half perfectly and is
  useless, and one that attaches every record to a nearest match satisfies the
  second while inventing the attribution — which is the failure §7a refuses for
  `unattributed`, one level out.

- **The emitted config needs a control that `async` is never written true.**
  *Added 2026-08-19.* §2's round 3b measured an `async: true` hook killed with
  its start written and its end never — silent non-delivery arriving through the
  mechanism installed to prevent it, one boolean away. Assert on the **emitted
  bytes**, not on a constant in the source: what ships is what install writes.
  Paired against a fixture that does declare `async: true` and is observed
  losing the record, so the control's premise is exercised rather than assumed.

- **The atomic replacement needs its negative half, and that half is the
  control.** *Added 2026-08-19.* A reader spinning on a target through many
  replacements and seeing only whole contents proves nothing on its own: a
  reader too slow to catch anything reports exactly that. So the **identical**
  reader runs against `std::fs::write` and **must** catch it between the
  truncate and the write. It does — `Empty` — and that is what licenses the
  clean sweep beside it. `the_truncating_write_is_caught_mid_replacement` and
  `a_replace_is_never_observed_partial`, both in the ordinary test job, so the
  claim is three-platform rather than read off documentation.

  **And a third asserts the temp file lands beside the target**, by watching the
  directory rather than by reading the implementation — a rename across volumes
  is a copy plus a delete, which puts the window back.

- **DECIDED, and it replaces the third-state proposal: prunability is not
  derivable from event content.** *Added 2026-08-19.* Round 3e proposed *ended
  at least once, reopenable*. Checked before building: **`reopenable` is always
  true** — a session two hours old was resumed from a different directory, and
  nothing in the payload bounds it. A variant whose predicate is always true
  distinguishes nothing. So control (f) is asserting a claim the payload cannot
  support, and what replaces it has to come from outside the events — file age,
  or an explicit user action — or prunability is not offered at all.

  **The cost is bounded structurally rather than by §8 staying open**: no
  `FileOp::Delete` exists, so nothing can act on a wrong label. The repair is
  owed because the label **claims what it does not know**, which is constraint 5
  — and that reason survives a display shipping, where *"nothing renders it"*
  would not.

- **The trigger says what it does not measure.** *Added 2026-08-19, and the
  definition is deliberately unchanged.* ADR-0008 §9's gate measures the
  **skip-path hazard**: controls that depend on an external tool and can
  silently skip when it is missing. That is a real hazard and the number is a
  fair proxy for it. It has never measured *"how many controls exist"*, and two
  rounds of controls landing outside it made that look like a fault in the gate
  rather than a fault in the reading. **Nobody should read 5 of 7 as "controls
  are stable."** Both numbers are printed in one invocation; only the second is
  a gate.

- **THE TIMING-DEPENDENT CONTROLS, ENUMERATED AND DISPOSITIONED.** *Added
  2026-08-19.* One control was found firing at **one red in six runs** — the
  sampling exception argued for and approved two rounds earlier. **The broken
  gate could not see any red, so the rest were not green, they were unmeasured**,
  and a small clean sample bounds nothing: the same suite went eight for eight
  on another tip while carrying the same flake.

  **The method, so the list is bounded rather than a hunt:** every control whose
  outcome depends on *when* two things happen — a `Duration`, an `Instant`, an
  `elapsed`, a `sleep`, or a second thread or process observed from the first.
  Across both crates' integration targets that is the following, complete:

  | control | disposition |
  | --- | --- |
  | `the_truncating_write_really_does_pass_through_an_empty_file` | **repaired by construction** — the window is built from `File::create` + `write_all` with the observation between, not sampled for. Was `..._is_caught_mid_replacement`, one red in six. |
  | `an_observer_can_see_a_partial_write_on_a_live_file` | **repaired by construction** — a file handshake, not a sleep. Residual: bounded waits (600 × 10 ms) that fail on timeout rather than hanging. Declared. |
  | `the_installed_timeout_is_what_the_rule_derives` | **repaired** — it was a **threshold on a noisy measurement**, asserting on a max over ten. Measured: twelve batches gave steady-state maxima of 14.7–18.7 ms and one of **48.8 ms**, against the 50 ms at which the assertion flips. Now asserts on **p90 and prints the max**. |
  | `a_replace_is_never_observed_partial` | **sampling, and one-sided** — it can only fail by *observing* a bad state, never by missing one, so it cannot go red without a defect. Its premise moved off the reader onto the writer. |
  | `a_cold_hook_invocation_is_measured_and_reported_per_platform` | **sampling with thresholds, declared** — 10 s against a ~15 ms steady state and 30 s against a ~1.2 s cold start. Roughly 500× and 25× headroom; no red observed. |
  | `two_writes_never_derive_the_same_temporary_name`, `the_temporary_file_is_derived_beside_the_target_and_leaves_no_residue` | **not timing-dependent** — threads exercise them, but every assertion is on a derivation or on final content. |
  | `ignore_state_git.rs`, `prompts_listing.rs` | **not timing-dependent** — a 30 s subprocess timeout is a bound on a hang, not an assertion about elapsed time. |

  **And the scratchpad instruments are outside the gate**, which is why they are
  not on this list: `hook-overlap*.js` and `kill-midwrite.js` sample by nature,
  nothing runs them automatically, and the overlap control **refuses rather than
  retries** when it does not fire.

  **The residual after all of it:** two controls still sample, both declared,
  both one-sided or with orders-of-magnitude headroom. **No rate is claimed for
  either** — see the note on estimation below.

- **A FLAKE RATE FROM SIX RUNS IS NOT A RATE.** *Added 2026-08-19.* The
  `1 in 6` above was used inside an argument that turns on frequency, and six
  runs does not support a frequency estimate. **The conclusion does not need
  it**: any non-zero non-deterministic red carries the cost of training people
  to ignore the colour, which is what a green proving nothing costs, arriving
  from the other side. So the label is **one red in six runs; the rate is not
  estimated** — and the same for every count in the table above.

- **The measurement instruments are ON-DEMAND, and that is a capability rather
  than a proof.** *Added 2026-08-19.* `scratchpad/hook-{collect,fixture,probe,
  variants,overlap,overlap-control}.js` are committed for the reason `probe.js`
  is (ADR-0002 §7): an instrument rebuilt from a description is a new instrument
  with the old one's name. **Nothing runs them.** They are not in CI, they need a
  real Claude Code binary and a live session, and every number they produced is
  a fact as of the build named beside it. A later reader must not treat their
  presence as continuous verification — the ADR's measurements go stale on
  upstream's schedule (ADR-0005 §10 rule 4a) and these files shorten the
  re-measurement, they do not perform it.

## Consequences

**Easier:** attribution is solved and needs nothing invented — three independent
routes, all inside the payload. The nested-fixture caveat costs nothing because
the design does not read the environment.

**Harder:** the honest answer is two facts rather than one, and the interesting
distinctions inside "alive and quiet" are not available from anything measured
here. A monitor that says less than a user wants is the price of not inventing
the rest.

**Harder again, after round 2, and this is a widening rather than a detail.**
*Added 2026-08-17.* "Alive and quiet" holds **three** cases with no
discriminator between them — executing, waiting for approval, and finished after
a denial — where round 1 recorded one plus a hedge. And the ambiguity runs in
both directions: silence overstates stoppage, an unclosed event overstates
activity. **Everything built here so far guards the silent direction only.** The
product consequence is that a monitor built on this record can report *that* an
agent reported something and *when*, and cannot report what it is doing now.

**Trade-off accepted:** hooks require installation, so coverage is opt-in and
partial by construction. The alternative — inferring state from liveness or file
timestamps — covers everything and is wrong in a way the user cannot see, which
is the trade this project has consistently refused.

**Not decided here, deliberately:** the open questions in §8, revised after round
2. Two are blocked on measurement that cannot be taken from this environment or
has not been taken — the interactive-TUI path, and `PostToolBatch`'s semantics
under parallel tool calls — and one is display design, which §6 constrains and
does not settle. **Transport is no longer among them**, and its absence from the
earlier list is recorded in §7a rather than quietly repaired, because a list of
open questions that reads as complete and is not is the same failure this
document is about.
