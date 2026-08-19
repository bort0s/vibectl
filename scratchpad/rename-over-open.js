#!/usr/bin/env node
'use strict';

/*
 * rename-over-open.js — can a rename replace a file another process holds open?
 *
 * ADR-0011 §7b installs while Claude Code is running. That is the NORMAL case,
 * not an edge: a user runs `vibe monitor install` from a terminal beside the
 * agent they want monitored. If a rename-over fails when the destination is
 * open, install fails exactly when it is meant to be used.
 *
 * `cache.rs` has carried the comment *"Windows will not rename onto an existing
 * file in every case"* since P0, with a fallback that DELETES the destination
 * and retries. That comment was read, not measured, and the fallback puts back
 * the window the rename exists to remove — with a `remove_file`, in a tool whose
 * second constraint is enforced by there being no delete.
 *
 * THREE HOLDERS, because "held open" is not one thing on Windows. The sharing
 * mode is the HOLDER's choice, not ours, and it decides the answer:
 *
 *   none    FileShare.None             nothing else may touch it
 *   read    FileShare.Read             readers welcome, no delete
 *   delete  FileShare.ReadWrite,Delete what Rust's own File::open asks for
 *
 * The third is what a reader written in Rust imposes; the first is the worst
 * case any program can impose on us.
 *
 * THE HANDSHAKE IS FILES, NOT SLEEPS, so the result does not depend on winning a
 * race (ADR-0002 §7). And an EBUSY when this script tries to read the target is
 * not an error to work around — it is the proof that the holder really took an
 * exclusive handle, so it is waited out rather than suppressed.
 *
 * usage: node rename-over-open.js <vibeRepoDir> <workDir>
 */

const { execFileSync, spawnSync, spawn } = require('child_process');
const fs = require('fs');
const path = require('path');

const [, , repo, work] = process.argv;
if (!repo || !work) {
  process.stderr.write('usage: node rename-over-open.js <vibeRepoDir> <workDir>\n');
  process.exit(4);
}
fs.mkdirSync(work, { recursive: true });

const MODES = [
  ['none', "'None'"],
  ['read', "'Read'"],
  ['delete', "'ReadWrite, Delete'"],
];

const ps = (p) => p.split('\\').join('\\\\');

function waitFor(p, ms) {
  const until = Date.now() + (ms || 15000);
  while (Date.now() < until) {
    if (fs.existsSync(p)) return true;
  }
  return false;
}

function readWhenFree(p) {
  const until = Date.now() + 15000;
  while (Date.now() < until) {
    try {
      return fs.readFileSync(p, 'utf8').trim();
    } catch (e) {
      if (e.code !== 'EBUSY') return '<' + e.code + '>';
    }
  }
  return '<still held>';
}

for (const [name, share] of MODES) {
  const target = path.join(work, 't-' + name + '.json');
  fs.writeFileSync(target, '{"old":true}\n');
  for (const suffix of ['.held', '.release']) {
    try { fs.rmSync(target + suffix, { force: true }); } catch (e) {}
  }

  const script =
    "$ErrorActionPreference='Stop'\n" +
    "$f=[System.IO.File]::Open('" + ps(target) + "','Open','Read'," + share + ")\n" +
    "New-Item -ItemType File -Path '" + ps(target + '.held') + "' -Force | Out-Null\n" +
    "while (-not (Test-Path '" + ps(target + '.release') + "')) { Start-Sleep -Milliseconds 20 }\n" +
    '$f.Close()\n';

  const child = spawn('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
    stdio: 'ignore',
  });

  if (!waitFor(target + '.held')) {
    process.stdout.write('  ' + name.padEnd(7) + ' SETUP FAILED — no handle was taken\n');
    child.kill();
    continue;
  }

  let verdict;
  try {
    execFileSync(
      'cargo',
      ['run', '--release', '-q', '-p', 'vibe-core', '--example', 'atomic_replace_once',
        '--', target, '{"new":true}'],
      { cwd: repo, stdio: ['ignore', 'pipe', 'pipe'] }
    );
    verdict = 'RENAMED OVER THE OPEN FILE';
  } catch (e) {
    const err = String(e.stderr || e.message).trim().split('\n').pop();
    verdict = 'REFUSED: ' + err;
  }

  fs.writeFileSync(target + '.release', '1');
  const after = readWhenFree(target);
  process.stdout.write('  ' + name.padEnd(7) + ' ' + verdict + '\n            file now: ' + after + '\n');
}

// And the real-world proxy: does Claude Code's own settings read block a
// replacement? A named claim about a named program, rather than about
// PowerShell's sharing modes.
process.stdout.write('\nreal-world proxy: `claude doctor` reading settings while a replace runs\n');
const proxy = path.join(work, 'proxy');
fs.mkdirSync(path.join(proxy, '.claude'), { recursive: true });
const settings = path.join(proxy, '.claude', 'settings.json');
fs.writeFileSync(settings, '{\n  "model": "opus"\n}\n');
const claude = process.env.VIBE_CLAUDE_EXE;
if (!claude || !fs.existsSync(claude)) {
  process.stdout.write('  skipped: set VIBE_CLAUDE_EXE to the claude binary\n');
} else {
  const doctor = spawn(claude, ['doctor'], { cwd: proxy, stdio: 'ignore' });
  let attempts = 0, refused = 0, lastErr = '';
  const until = Date.now() + 8000;
  while (Date.now() < until && doctor.exitCode === null) {
    attempts++;
    const r = spawnSync(
      'cargo',
      ['run', '--release', '-q', '-p', 'vibe-core', '--example', 'atomic_replace_once',
        '--', settings, '{"model":"opus"}'],
      { cwd: repo, stdio: ['ignore', 'pipe', 'pipe'] }
    );
    if (r.status !== 0) { refused++; lastErr = String(r.stderr).trim().split('\n').pop(); }
  }
  process.stdout.write(
    '  ' + attempts + ' replacements while `claude doctor` ran, ' + refused + ' refused' +
    (refused ? ' (last: ' + lastErr + ')' : '') + '\n'
  );
}
