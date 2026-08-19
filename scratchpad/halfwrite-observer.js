#!/usr/bin/env node
'use strict';

/*
 * halfwrite-observer.js — can the observer see a partial state on a LIVE file?
 *
 * `kill-midwrite.js` classifies a file after the process is gone, and its
 * positive control was a file truncated on purpose — a STATIC file, held open
 * by nobody. The target is a file a live process is mid-write on, and on NTFS
 * an observer reading a file another process holds open for writing may see a
 * cached view, be denied, or see the size update only at completion. A control
 * on the static case proves the CLASSIFIER recognises a torn file; it proves
 * nothing about whether the OBSERVER can see one being made.
 *
 * That distinction decides what a sweep of zeros means. If the observer is
 * blind while the writer holds the handle, then "no torn state at any delay" is
 * a fact about the observer and says nothing about the subject.
 *
 * So this constructs the state on purpose, with a KNOWN input: a writer that
 * appends half a record, waits, then appends the rest. During the wait the file
 * IS half a record and the writer IS holding it open. If the observer cannot
 * see that, it cannot see the thing the sweep was looking for.
 *
 * Paired in both directions: the same observer reads the file after the writer
 * has finished and must see the whole record, so "sees half" is not satisfied
 * by an observer that reports half unconditionally.
 *
 * usage: node halfwrite-observer.js <dir> [holdMs]
 * exit:  0 the observer sees the live half state; 3 it does not, and any
 *        mid-write zero measured with it is uncontrolled; 4 usage.
 */

const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');

const dir = process.argv[2];
const holdMs = Number(process.argv[3] || 1500);
if (!dir) {
  process.stderr.write('usage: node halfwrite-observer.js <dir> [holdMs]\n');
  process.exit(4);
}
fs.mkdirSync(dir, { recursive: true });

const target = path.join(dir, 'live-half.jsonl');
try { fs.rmSync(target, { force: true }); } catch (e) {}

const WHOLE = '{"v":"1","identity":"ident","session":"live","payload":"x"}\n';
const HALF = WHOLE.slice(0, Math.floor(WHOLE.length / 2));
const REST = WHOLE.slice(HALF.length);

// The writer is a separate PROCESS, not this one: a same-process read would
// bypass exactly the sharing question being asked.
const writerSrc = `
const fs = require('fs');
const target = ${JSON.stringify(target)};
const half = ${JSON.stringify(HALF)};
const rest = ${JSON.stringify(REST)};
const fd = fs.openSync(target, 'a');
fs.writeSync(fd, half);
const until = Date.now() + ${holdMs};
while (Date.now() < until) {}
fs.writeSync(fd, rest);
fs.closeSync(fd);
`;
const writerFile = path.join(dir, '_writer.js');
fs.writeFileSync(writerFile, writerSrc);

function observe(what) {
  let bytes = -1, text = '', err = null;
  try { text = fs.readFileSync(target, 'utf8'); bytes = Buffer.byteLength(text); }
  catch (e) { err = e.code; }
  const whole = bytes > 0 && text.endsWith('\n');
  process.stdout.write(
    `  ${what.padEnd(28)} bytes=${String(bytes).padStart(4)}  whole=${whole}` +
    (err ? `  ERROR=${err}` : '') + '\n'
  );
  return { bytes, whole, err };
}

const child = spawn(process.execPath, [writerFile], { stdio: 'ignore' });

setTimeout(() => {
  process.stdout.write(
    `a live writer is holding the handle, having written ${HALF.length} of ${WHOLE.length} bytes\n`
  );
  const during = observe('DURING the hold');

  child.on('exit', () => {
    setTimeout(() => {
      const after = observe('AFTER the writer exits');
      const sawHalf = during.err === null && during.bytes === HALF.length && !during.whole;
      const sawWhole = after.bytes === WHOLE.length && after.whole;
      process.stdout.write('\n');
      if (sawHalf && sawWhole) {
        process.stdout.write(
          'PASS — the observer sees a live partial state, and sees the whole\n' +
          'record once it is whole. A mid-write zero measured with this observer\n' +
          'is about the subject.\n'
        );
        process.exit(0);
      }
      process.stdout.write(
        'FAIL — the observer did NOT resolve the live half state' +
        (sawWhole ? '' : ' (and did not see the whole record either)') + '.\n' +
        'Any "no torn file" result measured through it belongs to the observer,\n' +
        'not to the subject.\n'
      );
      process.exit(3);
    }, 200);
  });
}, Math.floor(holdMs / 2));
