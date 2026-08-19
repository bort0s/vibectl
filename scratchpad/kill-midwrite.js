#!/usr/bin/env node
'use strict';

/*
 * kill-midwrite.js — what does a kill leave on disk for the REAL hook?
 *
 * ADR-0011 §7a admits exactly one corruption under one-writer-per-file:
 * truncation, from a crashed hook, a full disk or a killed process. §2's round
 * 3b measured that Claude Code's `timeout` DOES kill a hook (start written, end
 * never), so "a killed process" is not hypothetical. What was never measured is
 * the half that decides whether a short `timeout` is admissible at all:
 * **whether a kill can land INSIDE the append**, and what is on disk if it
 * does.
 *
 * The subject is `vibe monitor hook` itself, not a stand-in. It reads stdin,
 * builds one line, and issues one `write_all` on an append handle. A kill lands
 * either side of that call or, if `write_all` had to loop, inside it — and only
 * the third case can tear a record.
 *
 * TWO SIZES, BECAUSE THE ANSWER DEPENDS ON ONE. A realistic record is a few
 * kilobytes and goes out in a single `write` syscall, which a kill cannot
 * interrupt part-way. A record large enough to make `write_all` loop can be
 * torn. Measuring only the first would report "kills are clean" as a property
 * of the writer when it is a property of the size.
 *
 * THE CONSTRUCTION IS VERIFIED RATHER THAN ASSUMED. A run is only evidence
 * about mid-write kills if the kill actually landed mid-write, and that is
 * checkable after the fact: a file whose length is strictly between zero and
 * the whole line is a torn write, a file of length zero is a kill before it,
 * and a whole line is a kill after it. Every run reports which of the three it
 * was, so a race that was lost is visible instead of being counted as a clean
 * result. Nothing here retries.
 *
 * usage: node kill-midwrite.js <vibe.exe> <sinkDir> <payloadBytes> <killAfterMs> [n]
 */

const { spawn } = require('child_process');
const fs = require('fs');
const path = require('path');

const [, , vibe, sink, bytesArg, killMsArg, nArg] = process.argv;
if (!vibe || !sink || !bytesArg || !killMsArg) {
  process.stderr.write(
    'usage: node kill-midwrite.js <vibe.exe> <sinkDir> <payloadBytes> <killAfterMs> [n]\n'
  );
  process.exit(4);
}
const payloadBytes = Number(bytesArg);
const killMs = Number(killMsArg);
const n = Number(nArg || 1);

function payloadOf(session, bytes) {
  const head = `{"session_id":"${session}","hook_event_name":"SessionStart","filler":"`;
  const tail = '"}';
  const fill = 'x'.repeat(Math.max(0, bytes - head.length - tail.length));
  return head + fill + tail;
}

function classify(len, whole) {
  if (len === 0) return 'KILLED BEFORE THE WRITE (nothing on disk)';
  if (whole) return 'KILLED AFTER THE WRITE (a complete record)';
  return 'TORN — killed INSIDE the write';
}

async function once(i) {
  const session = 'k' + i;
  const payload = payloadOf(session, payloadBytes);
  const file = path.join(sink, `${session}__ident.jsonl`);
  try { fs.rmSync(file, { force: true }); } catch (e) {}

  const child = spawn(vibe, [
    'monitor', 'hook',
    '--identity', 'ident',
    '--sink', sink,
    '--contract', '1',
  ], { stdio: ['pipe', 'ignore', 'ignore'] });

  child.stdin.write(payload);
  child.stdin.end();

  const killed = await new Promise((res) => {
    const t = setTimeout(() => { child.kill('SIGKILL'); res(true); }, killMs);
    child.on('exit', () => { clearTimeout(t); res(false); });
  });
  // Let the OS settle the handle before measuring.
  await new Promise((r) => setTimeout(r, 150));

  let len = 0, text = '';
  try { text = fs.readFileSync(file, 'utf8'); len = Buffer.byteLength(text); } catch (e) {}
  const whole = len > 0 && text.endsWith('\n');
  const lines = text.length ? text.split('\n').filter(Boolean).length : 0;
  return {
    i, killedByUs: killed, len, whole, lines,
    verdict: classify(len, whole),
  };
}

(async () => {
  fs.mkdirSync(sink, { recursive: true });
  process.stdout.write(
    `payload ${payloadBytes} bytes, kill after ${killMs} ms, ${n} run(s)\n`
  );
  const counts = {};
  for (let i = 0; i < n; i++) {
    const r = await once(i);
    counts[r.verdict] = (counts[r.verdict] || 0) + 1;
    process.stdout.write(
      `  run ${String(r.i).padStart(2)}  killed-by-us=${r.killedByUs}  ` +
      `file=${String(r.len).padStart(10)} bytes  whole=${r.whole}  ${r.verdict}\n`
    );
  }
  process.stdout.write('\nsummary\n');
  for (const k of Object.keys(counts)) process.stdout.write(`  ${counts[k]}x  ${k}\n`);
  if (!Object.keys(counts).some((k) => k.startsWith('TORN'))) {
    process.stdout.write(
      '\nNO TORN WRITE WAS CONSTRUCTED IN THIS RUN.\n' +
      'That is not the same as "a kill cannot tear a record": it says the kill\n' +
      'landed either side of the write every time, at this size and this delay.\n'
    );
  }
})();
