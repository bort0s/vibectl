#!/usr/bin/env node
'use strict';
/*
 * Lifetime + census collector. TWO independent overlap instruments, because
 * ADR-0002 §7's channel rule means a single one cannot report its own zero.
 *
 *  1. INTERVALS. `performance.timeOrigin + performance.now()` at entry and at
 *     exit — epoch milliseconds, sub-millisecond resolution, comparable across
 *     processes. Two invocations overlap if their intervals intersect.
 *
 *  2. CENSUS. A marker file is created at entry and removed at exit, and the
 *     marker directory is listed while this process is alive. Anything else in
 *     that listing was alive at the same time as this process, which is the
 *     same question asked without any clock at all.
 *
 * THE PREVIOUS RUN'S ZERO WAS NOT REPORTABLE AND THIS IS THE REPAIR. The
 * recorded interval was 0.02-0.10 ms wide because the script does nothing, so
 * it excluded node's own startup and could not intersect anything. A DWELL is
 * added and disclosed:
 *
 *   - It cannot manufacture a false positive. If the parent dispatches hooks
 *     one at a time and waits for each, a slower hook delays the next one and
 *     never overlaps it, however long the dwell.
 *   - It can only raise sensitivity, and the recorded interval remains a SUBSET
 *     of the true process lifetime (node started before timeOrigin was read),
 *     so a detected overlap is real and a zero still establishes nothing on its
 *     own.
 *   - It makes the fixture MORE like the subject rather than less: a real vibe
 *     hook opens a file, appends and flushes, which is tens of milliseconds.
 *
 * The bound runs one way and it is the safe way.
 */
const fs = require('fs');
const path = require('path');
const start = performance.timeOrigin + performance.now();
const out = process.argv[2];
const label = process.argv[3] || '?';
const declared = process.argv[4] || '?';
const liveDir = path.join(path.dirname(out), 'live');
const marker = path.join(liveDir, process.pid + '-' + declared + '-' + label);

let stdin = '';
try { stdin = fs.readFileSync(0, 'utf8'); } catch (e) { stdin = '<<unreadable: ' + e.message + '>>'; }

let census = [];
try {
  fs.mkdirSync(liveDir, { recursive: true });
  fs.writeFileSync(marker, String(start));
} catch (e) { /* recorded below by its absence */ }

const DWELL_MS = 60;
const until = start + DWELL_MS;
let spins = 0;
while (performance.timeOrigin + performance.now() < until) {
  spins++;
  // census repeatedly through the dwell, so a short-lived neighbour is seen
  if (spins % 2000 === 0) {
    try { for (const f of fs.readdirSync(liveDir)) if (!census.includes(f)) census.push(f); } catch (e) {}
  }
}
try { for (const f of fs.readdirSync(liveDir)) if (!census.includes(f)) census.push(f); } catch (e) {}

const end = performance.timeOrigin + performance.now();
const line = JSON.stringify({
  label, declared, pid: process.pid, ppid: process.ppid,
  start_ms: start, end_ms: end, dur_ms: end - start,
  marker: path.basename(marker),
  census,
  stdin_len: stdin.length, stdin,
});
try {
  fs.appendFileSync(out, line + '\n');
} catch (e) {
  try { fs.appendFileSync(path.join(__dirname, 'COLLECTOR_ERROR.log'), String(e) + '\n'); } catch (_) {}
}
try { fs.unlinkSync(marker); } catch (e) {}
process.exit(0);
