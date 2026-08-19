#!/usr/bin/env node
'use strict';

/*
 * hook-probe.js — what does a hook do when a field is OMITTED?
 *
 * ADR-0011 §7a's contract pins execution properties. Install will write some of
 * them and omit the rest, and an omitted field is not "off" — it is whatever
 * the build defaults to, which is a property of the build and not of the
 * schema's prose. Omitting `shell` is not the same as writing `shell: false`:
 * if the default puts a shell in the channel, §7a chose the `args` exec form to
 * keep one out and the omission puts it back.
 *
 * TWO LINES PER INVOCATION, AND THAT IS THE POINT. A START line is appended
 * before the dwell and an END line after it. A hook killed by `timeout` writes
 * START and never END, so a kill is a VISIBLE PAIR rather than an absence — and
 * an absence is what a hook that never fired also produces. The two must not
 * share an observable.
 *
 * ARGV IS ECHOED AS RECEIVED, which is how the shell question is answered
 * without asking the OS anything: an argument written as `a$(whoami)b` arrives
 * literally when the process was spawned directly and mangled when a shell
 * parsed it. probe.js records argv for the same reason (ADR-0002 §7's channel
 * rule) and six of six discrepancies in one session were the channel.
 *
 * usage: <node> hook-probe.js <outFile> <label> <declaredEvent> [dwellMs] [marker...]
 */

const fs = require('fs');
const path = require('path');

const out = process.argv[2];
const label = process.argv[3] || '?';
const declared = process.argv[4] || '?';
const dwellMs = Number(process.argv[5] || 0);

function emit(phase, extra) {
  const line = JSON.stringify(Object.assign({
    phase, label, declared,
    pid: process.pid, ppid: process.ppid,
    t_ms: performance.timeOrigin + performance.now(),
    argv: process.argv.slice(2),
    // A shell in the channel leaves fingerprints its parent did not have.
    env_shlvl: process.env.SHLVL || null,
    env_underscore: process.env._ || null,
    env_ps_edition: process.env.POWERSHELL_DISTRIBUTION_CHANNEL || null,
  }, extra || {}));
  try {
    fs.mkdirSync(path.dirname(out), { recursive: true });
    fs.appendFileSync(out, line + '\n');
  } catch (e) {
    try { fs.appendFileSync(path.join(__dirname, 'PROBE_ERROR.log'), String(e) + '\n'); } catch (_) {}
  }
}

let stdin = '';
try { stdin = fs.readFileSync(0, 'utf8'); } catch (e) { stdin = ''; }
let payload = {};
try { payload = JSON.parse(stdin); } catch (e) {}

emit('start', {
  session: payload.session_id || null,
  agent: payload.agent_id || null,
  event: payload.hook_event_name || null,
  tool: payload.tool_name || null,
});

if (dwellMs > 0) {
  const until = performance.timeOrigin + performance.now() + dwellMs;
  while (performance.timeOrigin + performance.now() < until) { /* spin, no timer to be killed with */ }
}

emit('end', { dwell_ms: dwellMs });
process.exit(0);
