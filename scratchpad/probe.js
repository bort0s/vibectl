#!/usr/bin/env node
'use strict';

/*
 * probe.js — a search that cannot report a zero without a passing control in
 * the same invocation. An instrument, not a product: it lives in a scratchpad
 * and nothing in the crate links it.
 *
 * ADR-0002 §7 is the specification. Three properties it requires:
 *
 *  1. The result carries the proof that the mechanism ran. A pattern known to
 *     be present is searched in the SAME invocation, through the SAME code
 *     path, over the SAME bytes. If it matches nothing, the target counts are
 *     WITHHELD and the exit is nonzero — an empty result with no positive
 *     control is a skipped test wearing a green tick.
 *
 *  2. argv is echoed as received. Six channel failures in one session — a
 *     dropped "", a split quoted argument, a space-joined -ArgumentList, an
 *     MSYS-rewritten leading slash, a BOM ahead of a slash, a case-insensitive
 *     variable table — all arrived as facts about the subject. Printing what
 *     actually landed makes the channel visible rather than inferred.
 *
 *  3. Bytes are read as UTF-8 here and matched with a JS regex, so no shell
 *     locale decides what a character is.
 *
 * TWO REPAIRS, AND THE SECOND IS THE ONE THE FIRST DID NOT COVER.
 *
 *   Repair A — match the WHOLE TEXT, never line by line. The first version
 *   matched per line, so a phrase spanning a newline could never match. Its
 *   control passed throughout, because the control was a single-line string.
 *
 *   Repair B — a run of whitespace in the phrase matches ANY run of
 *   whitespace, newlines included. Repair A alone is not enough, and this is
 *   exactly where the second false zero came from: whole-text matching lets a
 *   pattern SPAN lines, it does not make the pattern ABLE to. A literal space
 *   in the phrase still cannot match the newline a hard-wrapped corpus put
 *   there.
 *
 * AND THE WIDENING IS BOUNDED, because a pattern matching everything is the
 * same failure with the sign flipped. The `\s+` widening requires at least one
 * whitespace character and crosses nothing else, so a genuinely absent phrase
 * still returns zero. That is asserted by a paired sabotage rather than
 * assumed.
 *
 * THE CONTROL'S OWN SHAPE IS CHECKED, because a control proves only the hazard
 * class it exercises. A single-line control cannot establish that a multi-line
 * target could have matched — the exact blindness that survived repair A. So
 * if any target returns ZERO while no control match crossed a line break, that
 * zero is NOT REPORTABLE and the exit is nonzero. The rule stops being
 * something to remember and becomes something the instrument enforces.
 *
 * EXIT CODES
 *   0  ran; every control matched; results are reportable
 *   2  a control matched nothing — DEAD CONTROL, all target counts withheld
 *   3  a target returned zero and no control match crossed a line break —
 *      that zero is not reportable
 *   4  usage error, unreadable corpus, or an uncompilable pattern
 *
 * A nonzero target count is self-proving (the pattern demonstrably matched),
 * which is why exit 3 gates on zeros only. Exit 2 withholds everything,
 * because a run whose control failed has established nothing about itself.
 */

const fs = require('fs');

const EX_OK = 0;
const EX_DEAD_CONTROL = 2;
const EX_UNSHAPED_ZERO = 3;
const EX_USAGE = 4;

const USAGE = [
  'usage: node probe.js --control <phrase> [--control <phrase>...]',
  '                     --target  <phrase> [--target  <phrase>...]',
  '                     <file> [<file>...]',
  '',
  'A phrase is a LITERAL. Every regex metacharacter in it is escaped; the one',
  'thing that is not literal is whitespace, where a run in the phrase matches',
  'any run in the corpus — including the newline a hard wrap put there.',
  '',
  'At least one control and one target are required. Give a control known to',
  'be hard-wrapped in the corpus, or a zero target cannot be reported.',
].join('\n');

function die(code, msg) {
  process.stderr.write(msg + '\n');
  process.exit(code);
}

/**
 * A literal phrase, whitespace-flexible. Repair B lives on the second line.
 *
 * Escape first, so a phrase containing `.` or `(` cannot become a pattern,
 * then widen whitespace. The order matters and is safe: none of the escaped
 * metacharacters is whitespace, so the widening cannot reach inside an escape
 * it just wrote.
 */
function phraseToRegex(phrase) {
  const escaped = phrase.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const flexible = escaped.replace(/\s+/g, '\\s+');
  try {
    return new RegExp(flexible, 'gu');
  } catch (e) {
    return die(EX_USAGE, 'probe: phrase does not compile: ' + phrase + '\n  ' + e.message);
  }
}

/** Whole-text matching. Repair A is the absence of any per-line loop here. */
function search(text, phrase) {
  const re = phraseToRegex(phrase);
  const hits = [];
  for (const m of text.matchAll(re)) {
    hits.push({ index: m.index, text: m[0], spansLine: /[\r\n]/.test(m[0]) });
  }
  return hits;
}

function lineOf(text, index) {
  let line = 1;
  for (let i = 0; i < index; i++) {
    if (text.charCodeAt(i) === 10) line++;
  }
  return line;
}

function main() {
  const argv = process.argv.slice(2);

  // Property 2, and it runs before anything can fail: whatever the channel did
  // to these arguments is on the page before a single byte is read.
  process.stdout.write('probe.js — argv as received\n');
  if (argv.length === 0) {
    process.stdout.write('  (no arguments)\n');
  }
  argv.forEach((a, i) => {
    process.stdout.write('  [' + i + '] ' + JSON.stringify(a) + '  (' + a.length + ' chars)\n');
  });
  process.stdout.write('\n');

  const controls = [];
  const targets = [];
  const files = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--control' || a === '--target') {
      const v = argv[++i];
      if (v === undefined) die(EX_USAGE, 'probe: ' + a + ' needs a phrase\n\n' + USAGE);
      (a === '--control' ? controls : targets).push(v);
    } else if (a === '-h' || a === '--help') {
      die(EX_USAGE, USAGE);
    } else {
      files.push(a);
    }
  }
  if (controls.length === 0 || targets.length === 0 || files.length === 0) {
    die(EX_USAGE, 'probe: need at least one --control, one --target and one file\n\n' + USAGE);
  }

  // The corpus, read once, as UTF-8, with a BOM reported rather than silently
  // eaten — an instrument that alters its input is the failure class this file
  // exists against.
  let corpus = '';
  process.stdout.write('corpus\n');
  for (const f of files) {
    let raw;
    try {
      raw = fs.readFileSync(f, 'utf8');
    } catch (e) {
      die(EX_USAGE, 'probe: cannot read ' + f + ': ' + e.message);
    }
    const bom = raw.charCodeAt(0) === 0xfeff;
    const text = bom ? raw.slice(1) : raw;
    const lines = text.split('\n').length;
    const crlf = /\r\n/.test(text);
    process.stdout.write(
      '  ' + f + '\n    ' + text.length + ' chars, ' + lines + ' lines, ' +
      (bom ? 'BOM PRESENT (stripped for matching)' : 'no BOM') + ', ' +
      (crlf ? 'CRLF' : 'LF') + '\n'
    );
    corpus += (corpus ? '\n' : '') + text;
  }
  process.stdout.write('\n');

  // Controls, in the same invocation, over the same bytes, through the same
  // `search`. Anything less and the zero below means nothing.
  process.stdout.write('controls (positive — must match)\n');
  let deadControl = false;
  let anyControlSpansLine = false;
  for (const phrase of controls) {
    const hits = search(corpus, phrase);
    const spanning = hits.filter((h) => h.spansLine).length;
    if (hits.length === 0) deadControl = true;
    if (spanning > 0) anyControlSpansLine = true;
    process.stdout.write(
      '  ' + (hits.length === 0 ? 'DEAD' : 'PASS') + '  ' +
      hits.length + ' match' + (hits.length === 1 ? '' : 'es') +
      ', ' + spanning + ' crossing a line break   ' + JSON.stringify(phrase) + '\n'
    );
  }
  process.stdout.write('\n');

  if (deadControl) {
    process.stdout.write('targets\n  WITHHELD — a control matched nothing.\n\n');
    process.stderr.write(
      'probe: DEAD CONTROL. A pattern known to be present matched zero times, so\n' +
      'this invocation has not established that the corpus was read, that the\n' +
      'encoding survived, or that the matcher runs. No target count is reported;\n' +
      'a zero from this run would be a skipped test wearing a green tick.\n'
    );
    process.exit(EX_DEAD_CONTROL);
  }

  process.stdout.write('targets\n');
  let unshapedZero = false;
  for (const phrase of targets) {
    const hits = search(corpus, phrase);
    if (hits.length === 0 && !anyControlSpansLine) {
      unshapedZero = true;
      process.stdout.write(
        '  ZERO NOT REPORTABLE  ' + JSON.stringify(phrase) + '\n' +
        '      no control match crossed a line break, so this run cannot\n' +
        '      distinguish "absent" from "the matcher is line-blind"\n'
      );
      continue;
    }
    const spanning = hits.filter((h) => h.spansLine).length;
    process.stdout.write(
      '  ' + hits.length + '  ' + JSON.stringify(phrase) +
      (spanning > 0 ? '   (' + spanning + ' crossing a line break)' : '') + '\n'
    );
    for (const h of hits.slice(0, 8)) {
      // Capped. An over-wide pattern can match hundreds of lines, and an
      // instrument that floods the terminal is one whose output nobody reads —
      // the span length is the finding, so it is printed instead of the span.
      const shown = h.text.length > 220
        ? JSON.stringify(h.text.slice(0, 200)) + ' … +' + (h.text.length - 200) + ' chars'
        : JSON.stringify(h.text);
      process.stdout.write(
        '      line ' + lineOf(corpus, h.index) + ': ' + shown + '\n'
      );
    }
    if (hits.length > 8) {
      process.stdout.write('      … ' + (hits.length - 8) + ' more\n');
    }
  }
  process.stdout.write('\n');

  if (unshapedZero) {
    process.stderr.write(
      'probe: A TARGET RETURNED ZERO AND NO CONTROL EXERCISED THE HAZARD.\n' +
      'Every control matched inside a single line, so none of them shows that a\n' +
      'phrase crossing a hard wrap could have matched — which is the blindness\n' +
      'that produced this instrument’s last two false zeros. Re-run with a\n' +
      'control phrase known to be hard-wrapped in this corpus.\n'
    );
    process.exit(EX_UNSHAPED_ZERO);
  }

  process.exit(EX_OK);
}

main();
