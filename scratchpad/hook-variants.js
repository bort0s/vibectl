#!/usr/bin/env node
'use strict';

/*
 * hook-variants.js — one settings file declaring several variants of the SAME
 * hook, so an omitted field and a written one are compared inside a single run.
 *
 * ADR-0002 §7's pairing rule, applied to a default: "the field is omitted"
 * means nothing on its own, because a hook that never fired and a hook that
 * fired with the default look identical from outside. Every variant here is
 * declared beside its pair, in one run, over one session — so the comparison is
 * between two observables produced by the same session rather than between two
 * runs that may have differed in some other way.
 *
 * ONE FILE, NOT TWO. §2 measured that both settings files fire, which doubles
 * every count. That is a real property and it is noise here, so these fixtures
 * declare hooks in `.claude/settings.json` only and the expected multiplicity
 * is one. Where the union matters it is measured elsewhere (see
 * `hook-fixture.js`, which deliberately writes both).
 *
 * SEVERAL EVENTS, BECAUSE A MATCHER MEANS A DIFFERENT THING ON EACH. A matcher
 * on `PreToolUse` filters tool names; on `SessionStart` it filters something
 * else, or nothing at all. Measuring a match-all on a tool event and then
 * installing it on a lifecycle event is a control proving one hazard class and
 * shipping against another — so the variant list is repeated per event, and
 * every label carries its event so the two can never merge in a tally.
 *
 * usage: node hook-variants.js <fixtureDir> <outFile> <event[,event...]> <variantsJsonFile>
 *
 * A variant is { label, extra?, matcher?, dwellMs?, extraArg?, commandString? }:
 *   extra          merged into the hook object — this is where `once`, `if`,
 *                  `shell`, `timeout`, `async`, `asyncRewake` go, and where
 *                  LEAVING ONE OUT is the measurement.
 *   matcher        hoisted to the enclosing matcher-group, which is where the
 *                  schema puts it — a different object from the hook.
 *   dwellMs        passed to hook-probe.js, so a timeout has something to kill.
 *   extraArg       one more argv element, for the literal-argument shell probe.
 *   commandString  declares the STRING form (no `args`), which is the variant
 *                  that asks whether a shell is in the channel at all.
 */

const fs = require('fs');
const path = require('path');

const [, , fixtureDir, outFile, eventArg, variantsFile] = process.argv;
if (!fixtureDir || !outFile || !eventArg || !variantsFile) {
  process.stderr.write(
    'usage: node hook-variants.js <fixtureDir> <outFile> <event[,event...]> <variantsJsonFile>\n'
  );
  process.exit(4);
}
const events = eventArg.split(',').map((e) => e.trim()).filter(Boolean);
const variants = JSON.parse(fs.readFileSync(variantsFile, 'utf8'));
const NODE = process.execPath;
const probe = path.join(fixtureDir, 'hook-probe.js');

function groupsFor(event) {
  const groups = [];
  for (const v of variants) {
    const label = events.length > 1 ? event + ':' + v.label : v.label;
    const hook = { type: 'command' };
    if (v.commandString) {
      hook.command = v.commandString
        .split('{NODE}').join(NODE)
        .split('{PROBE}').join(probe)
        .split('{OUT}').join(outFile)
        .split('{LABEL}').join(label);
    } else {
      hook.command = NODE;
      hook.args = [probe, outFile, label, event, String(v.dwellMs || 0)];
      if (v.extraArg) hook.args.push(v.extraArg);
    }
    Object.assign(hook, v.extra || {});
    const group = { hooks: [hook] };
    if (v.matcher !== undefined) group.matcher = v.matcher;
    groups.push(group);
  }
  return groups;
}

const hooks = {};
for (const e of events) hooks[e] = groupsFor(e);

fs.mkdirSync(path.join(fixtureDir, '.claude'), { recursive: true });
fs.writeFileSync(
  path.join(fixtureDir, '.claude', 'settings.json'),
  JSON.stringify({ hooks }, null, 2) + '\n'
);
process.stdout.write(
  'wrote ' + variants.length + ' variant(s) x ' + events.length + ' event(s)\n' +
  '  events: ' + events.join(', ') + '\n' +
  variants
    .map(
      (v) =>
        '  ' +
        v.label +
        '  ' +
        JSON.stringify(
          Object.assign({}, v.extra, v.matcher !== undefined ? { matcher: v.matcher } : {})
        )
    )
    .join('\n') +
  '\n'
);
