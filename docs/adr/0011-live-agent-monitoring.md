# ADR-0011: Live agent monitoring — agents report, and the last event is never terminal

## Status

**Scoped (2026-08-12), design open, no code.** P6's second feature. This records
what was measured, the one architectural decision taken in principle, and the
constraints that hold *whatever* the design turns out to be — the shape ADR-0009
used for the frontend, and for the same reason: "nothing recorded" and "no
constraints" are different things.

The decisions still open are listed in §7 as open, with what each would cost.
They are not decided here because they are not mine to decide.

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
live PID is not proof it is *the same* process — the honest reading pairs the PID
with something that identifies the run, and `session_id` is in the payload for
exactly that.

The composite that is actually supported by evidence is therefore two facts, kept
separate rather than collapsed: **what was last reported, and whether the
reporter still exists.** How many display states that yields, and what each is
called, is design and is open.

**What is not available from any measurement here:** a way to distinguish
*thinking*, *running a long tool*, and *waiting for approval* while the process
is alive and quiet. `PermissionRequest` and `Notification` exist and were never
observed firing; whether they close that gap is unmeasured and is the first thing
to measure when this is picked up.

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

### 7. Open, and not mine to decide

Listed with what each costs, so the next round starts with numbers.

1. **Where hook configuration lives, and who writes it.** `settings.json` is
   shared and versioned by the same argument ADR-0010 §3 makes for prompts;
   `settings.local.json` is per-machine and conventionally ignored. Both load
   (§2), so the choice is not about whether it works. The cost that makes this a
   real decision: **vibe writing hook config is not the singular user-initiated
   write ADR-0010 §3 permitted itself.** Hooks must stay in step with what vibe
   expects to receive, which is a consistency relationship between two artifacts
   — the shape ADR-0010 rejected for prompts. Whether that reasoning transfers is
   the question, and it may not: the second artifact here is configuration rather
   than a copy of content.
2. **Whether monitoring is opt-in per project.** Hooks only fire where they are
   installed, so an uninstrumented project is invisible — and per §6 invisible
   must not render as stopped.
3. **Whether `PermissionRequest` and `Notification` are used**, which cannot be
   decided before they are measured.

### 8. Negative-control obligations for whatever gets built

- **The third state needs a paired control**, per ADR-0002 §7's rule against
  one-sided ones: an agent killed mid-tool must render as the third state, **and
  a live agent with the same last event must render as working**. A build that
  reports the third state unconditionally satisfies the first half perfectly.
- **The dedupe needs a control that both files are declaring hooks**, or it tests
  nothing — the union in §2 is the precondition, and a fixture with hooks in one
  file only would pass while the defect ships.
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

**Not decided here, deliberately:** every question in §7, plus the shape of the
display. Those need one more measurement round and one decision that is the
owner's.
