# ADR-0011: Live agent monitoring — agents report, and the last event is never terminal

## Status

**Scoped (2026-08-12), design open, no code.** P6's second feature. This records
what was measured, the one architectural decision taken in principle, and the
constraints that hold *whatever* the design turns out to be — the shape ADR-0009
used for the frontend, and for the same reason: "nothing recorded" and "no
constraints" are different things.

The decisions still open are listed in §8, and both are blocked on measurement
or are display design rather than on anyone's preference.

## Context

The feature is seeing which Claude Code instances are running, what they are
doing, and whether they have stopped. It shares a measurement round with
[ADR-0010](0010-per-project-prompts.md) and none of its decisions.

Versions, because a property belongs to a build:

| | |
| --- | --- |
| Claude Code | **2.1.228**, native binary bundled in the VSCode extension |
| OS | Windows 10 Pro 19045; node v24.16.0 |
| Codex | **not installed — nothing measured**, per ADR-0010 §1 |

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

**Six event types observed in one run:** `SessionStart`, `UserPromptSubmit`,
`PreToolUse`, `PostToolUse`, `Stop`, `SessionEnd`.

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
  resolution and stable across reads — and **not universally readable**: one
  process in a four-row sample returned an empty start time, because the datum
  is access-controlled. The agent runs as the same user, so it should be
  readable, but *"start time unavailable"* is a third outcome and must not
  collapse into either alive or gone. That is the same `NotAttempted` line one
  level down.
- **Or one crate covering all three**, which is a dependency added to a library
  every embedder links — the cost ADR-0008 §4 weighed when it declined 18 crates
  for a TLS stack. Not weighed here, because that is a decision for the diff.

**Without something of that shape, liveness is an inference wearing the costume
of a fact**, and it belongs in the same category as the silence this whole design
refuses to read.

**How much is actually unresolved while a process is alive and quiet — narrower
than an earlier draft claimed.** The measured pairs close part of it: `PreToolUse`
carries `tool_use_id` and `PostToolUse` closes it with the same value
(`toolu_01AfLoKWUmCsA7LtkYkBNsaS`, observed in both), and the mid-tool kill left
an opened id with no close. So **an unmatched `tool_use_id` with the reporter
alive is a tool in flight — a reported fact, not an inference from silence, and
it is not thinking.**

What remains is one distinction, possibly two, not three:

- **Inside an open `tool_use_id`:** a slow tool and an agent waiting for approval
  are indistinguishable — *if* approval is requested within the tool window,
  which is **unmeasured**. Headless invocation auto-denies, so no fixture here
  produced an approval prompt.
- **With no open `tool_use_id` and the reporter alive:** thinking or streaming.

`PermissionRequest` and `Notification` were never observed firing and are what
would close the first bullet. Measuring them is the first thing to do when this
is picked up, and the second is establishing whether an approval wait sits inside
the tool window at all.

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
nothing has to be remembered or checked on the side. It also inherits that rule's
limit, and the limit must be stated or it will be over-read — **a delivered
`SessionStart` proves the wiring worked at session start, not that it is working
now.** A hook removed mid-session, or one that fails on a later event only, is
not covered. That is the environment-shaped hole closed and the code-shaped one
left open, exactly as ADR-0002 §7 records for the original.

It follows that **monitoring is opt-in per project** — hooks fire only where
installed — and that an uninstrumented project renders as unknown rather than
idle, per §6.

### 8. Open, and not mine to decide

1. **Whether `PermissionRequest` and `Notification` are used**, which cannot be
   decided before they are measured (§5).
2. **The display shape and the state names.** How many states the two facts of §5
   yield, and what each is called, is design; §6 constrains it and does not
   settle it.

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

## Consequences

**Easier:** attribution is solved and needs nothing invented — three independent
routes, all inside the payload. The nested-fixture caveat costs nothing because
the design does not read the environment.

**Harder:** the honest answer is two facts rather than one, and the interesting
distinctions inside "alive and quiet" are not available from anything measured
here. A monitor that says less than a user wants is the price of not inventing
the rest.

**Trade-off accepted:** hooks require installation, so coverage is opt-in and
partial by construction. The alternative — inferring state from liveness or file
timestamps — covers everything and is wrong in a way the user cannot see, which
is the trade this project has consistently refused.

**Not decided here, deliberately:** the two questions in §8. One is blocked on
measuring `PermissionRequest` and `Notification`; the other is display design,
which §6 constrains and does not settle.
