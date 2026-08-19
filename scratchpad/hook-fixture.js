#!/usr/bin/env node
'use strict';

/*
 * hook-fixture.js — writes the two settings files a hook measurement needs.
 *
 * usage: node hook-fixture.js <fixtureDir> <outFile> <settingsSchema.json> [extraEventName]
 *
 * `hook-collect.js` must sit in <fixtureDir>. Pass an extra name to plant a
 * deliberately bogus event and check that `claude doctor` reports it — the
 * name-registration control, without which a zero for some event is a claim
 * about this fixture's spelling rather than about the build.
 *
 * Both declare a hook for EVERY name the build's own schema accepts, so a name
 * that does not fire is a fact about firing rather than about what was
 * installed. Both files declare the same set, which makes §2's union a
 * CONSTRUCTED precondition rather than an inherited claim — §9's dedupe
 * obligation requires exactly that, and a fixture with hooks in one file only
 * would pass while proving nothing.
 */
const fs = require('fs');
const path = require('path');
const [, , fixtureDir, outFile, schemaPath, extraName] = process.argv;
const schema = JSON.parse(fs.readFileSync(schemaPath, 'utf8'));
const names = schema.properties.hooks.propertyNames.anyOf[0].enum.slice();
if (extraName) names.push(extraName);
const NODE = process.execPath;
const collector = path.join(fixtureDir, 'hook-collect.js');
function build(label) {
  const hooks = {};
  for (const n of names) {
    hooks[n] = [
      {
        hooks: [
          {
            type: 'command',
            command: NODE,
            args: [collector, outFile, label, n],
            timeout: 30,
          },
        ],
      },
    ];
  }
  return { hooks };
}
fs.mkdirSync(path.join(fixtureDir, '.claude'), { recursive: true });
fs.writeFileSync(path.join(fixtureDir, '.claude', 'settings.json'), JSON.stringify(build('A'), null, 2) + '\n');
fs.writeFileSync(path.join(fixtureDir, '.claude', 'settings.local.json'), JSON.stringify(build('B'), null, 2) + '\n');
console.log('declared ' + names.length + ' event names in each of two settings files');
console.log('node: ' + NODE);
console.log('collector: ' + collector);
console.log('out: ' + outFile);
