#!/usr/bin/env node
'use strict';

/*
 * hook-overlap-control.js — the positive control for `hook-overlap.js`, in the
 * SAME SHAPE as the target.
 *
 * WHY THIS FILE EXISTS. The target question is intra-agent, SAME-KEY overlap:
 * two invocations sharing (session_id, agent_id, declared identity) alive at
 * once. A run whose only overlaps are cross-identity proves the analyser can
 * pair invocations ACROSS files and proves nothing about the grouping path the
 * same-key answer travels — a bug in that path produces cross-identity
 * overlaps and a same-key zero, which is exactly the picture a healthy build
 * also produces. A control proves only the hazard class it exercises.
 *
 * So this spawns invocations that share ONE key, on purpose, and the analyser
 * must report them as same-key. Nothing here is cross-identity.
 *
 * AND IT IS SPAWNED FROM ONE PARENT RATHER THAN FROM A SHELL. Two `node` calls
 * backgrounded from a shell start 10-100 ms apart because each pays its own
 * interpreter startup before the recorded interval opens, and the collector's
 * window is 60 ms — so the control fires or misses by luck. That is not a
 * control, it is a coin flip that reads as one. Spawning them together from a
 * process that is already warm removes the interpreter startup from the skew.
 *
 * IT STILL REFUSES RATHER THAN RETRIES. If the spawns land far enough apart
 * that nothing overlaps, this exits nonzero and says the control did not fire.
 * A control that loops until it succeeds cannot tell "the instrument works"
 * from "the twentieth attempt got lucky".
 *
 * usage: node hook-overlap-control.js <collector.js> <outFile> [n]
 * exit:  0 the analyser must now report same-key overlap; 3 the control did not
 *        fire and nothing is established; 4 usage.
 */

const { spawn } = require('child_process');
const fs = require('fs');
const path = require('path');

const collector = process.argv[2];
const out = process.argv[3];
const n = Number(process.argv[4] || 4);
if (!collector || !out) {
  process.stderr.write('usage: node hook-overlap-control.js <collector.js> <outFile> [n]\n');
  process.exit(4);
}

// One key for every child: same session, same agent, same declared identity.
const PAYLOAD = JSON.stringify({
  session_id: 'CONTROL_SESSION',
  agent_id: 'CONTROL_AGENT',
  hook_event_name: 'PreToolUse',
  tool_name: 'ControlTool',
});
const IDENTITY = 'control-identity';

try { fs.mkdirSync(path.dirname(out), { recursive: true }); } catch (e) {}
try { fs.mkdirSync(path.join(path.dirname(out), 'live'), { recursive: true }); } catch (e) {}

const children = [];
for (let i = 0; i < n; i++) {
  const c = spawn(process.execPath, [collector, out, IDENTITY, 'PreToolUse'], {
    stdio: ['pipe', 'inherit', 'inherit'],
  });
  c.stdin.end(PAYLOAD + '\n');
  children.push(new Promise((res) => c.on('exit', res)));
}

Promise.all(children).then(() => {
  const rows = fs.readFileSync(out, 'utf8').split('\n').filter(Boolean).map(JSON.parse)
    .filter((r) => r.label === IDENTITY);
  let pairs = 0, widest = 0;
  for (let i = 0; i < rows.length; i++) {
    for (let j = i + 1; j < rows.length; j++) {
      const ov = Math.min(rows[i].end_ms, rows[j].end_ms) - Math.max(rows[i].start_ms, rows[j].start_ms);
      if (ov > 0) { pairs++; if (ov > widest) widest = ov; }
    }
  }
  process.stdout.write(
    'control: ' + rows.length + ' invocations under ONE key (' +
    'CONTROL_SESSION, CONTROL_AGENT, ' + IDENTITY + ')\n' +
    'control: ' + pairs + ' same-key overlapping pairs, widest ' + widest.toFixed(1) + ' ms\n'
  );
  if (pairs === 0) {
    process.stderr.write(
      'CONTROL DID NOT FIRE. The spawns did not land close enough to overlap, so\n' +
      'this invocation has not shown that the same-key path can report an overlap\n' +
      'at all. Any same-key zero measured alongside it is uncontrolled.\n'
    );
    process.exit(3);
  }
  process.exit(0);
});
