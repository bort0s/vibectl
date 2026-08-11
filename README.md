# vibectl

**A registry for the half-finished projects you already have.**

You have 30 project directories. You remember what maybe six of them do. `vibe`
indexes the rest — inferring stack, git state, and deploy target from what is
already on disk — and keeps a small manifest next to each one so you (and your
agents) can pick any of them back up without archaeology.

> **Status: pre-alpha, under active development.** Nothing is published to
> crates.io yet.
>
> Working today: `vibe new`, `vibe scan`, `vibe list`, `vibe show`, `vibe sync`,
> `vibe archive` / `vibe unarchive`, and `vibe agents`
> (`update`/`list`/`status`/`add`/`remove`/`sync`), with `--json` on reads and
> `--dry-run` on writes. Manifests round-trip through `toml_edit`, so your
> comments and any keys this build does not recognise survive editing.
>
> Not built yet: `vibe render` (P4), `gh` integration in `vibe new` (P5), and
> optional AI enrichment (P6).

## The idea

Most project tooling assumes you are starting from zero. This one assumes you
are starting from a mess.

```console
$ vibe scan ~/projects
macroring  /home/you/projects/macroring
  stack     node@22
  uses      react@19, vite@5
  services  supabase, vercel
  remote    github.com/you/macroring
  env       SUPABASE_ANON_KEY, SUPABASE_URL

mystery  /home/you/projects/mystery
  stack     —
  remote    —

2 project(s) in 43ms

$ vibe list
NAME       STACK    STATUS   COMMIT      REMOTE
macroring  node@22  active   2026-08-08  github.com/you/macroring
mystery    —        idea     —           —
```

The second project has a `Dockerfile` saying `FROM python:3.11` and nothing
else. It reads as obviously Python to a human, so `vibe` says nothing — there is
no `pyproject.toml`, no `requirements.txt`, and no `.py` file. An empty field is
the honest answer, and `--suggestions` shows what was found but not written.

`vibe scan` is the point of the tool. Scaffolding (`vibe new`) exists because it
would be strange if it didn't, and is kept deliberately minimal.

## What this is not

- **Not a competitor to `cookiecutter`, `degit`, or `npm create`.** Scaffolding
  is table stakes here, not the product.
- **Not a replacement for `CLAUDE.md` / `AGENTS.md`.** The manifest is the source
  of truth; those files are generated *outputs* (`vibe render`).
- **Not a GUI.** CLI only in v1. The core is a separate crate so a desktop
  frontend could reuse it later, but no UI is being built.

## Commands (planned)

| Command | Does |
| --- | --- |
| `vibe new <name>` | Scaffold a project, write a manifest, optionally create the repo |
| `vibe scan <path>` | Index existing projects, infer manifests |
| `vibe list` | Table: name, stack, status, last commit, remote, deploy |
| `vibe show <name>` | Full manifest dump, ready to paste at an agent |
| `vibe sync [<name>]` | Re-read git / `package.json` / etc., update manifests |
| `vibe render <target>` | Generate `CLAUDE.md`, `AGENTS.md`, or `README.md` |
| `vibe archive <name>` | Take a project off your desk. Never deletes anything. |
| `vibe unarchive <name>` | Put it back. |
| `vibe agents <sub>` | Install agent definitions from a git-backed store into `.claude/agents/` |

`--json` on every read command, and on any `--dry-run` — a plan is a proposal to
be inspected before it runs, so it is machine-readable too. `--dry-run` on every
write command.

`archive` is orthogonal to `status`: it sets `archived = true` and leaves
`status` alone, so a *shipped* project can be filed away without being relabelled
*dead*, and `unarchive` restores the exact prior state rather than guessing one.
`vibe list` hides archived projects unless you pass `--all`.

## The manifest

One per project, at `.vibe/project.toml`, committed alongside your code.

```toml
[project]
name = "macroring"
description = "Mobile-first PWA for nutrition tracking"
status = "active"          # idea | active | paused | shipped | dead
created = "2026-03-12"

[stack]
runtime = "node@22"
frameworks = ["react@19", "vite", "typescript"]
services = ["supabase", "vercel"]

[repo]
remote = "github.com/user/macroring"
visibility = "private"

[deploy]
url = "https://macroring.vercel.app"
env_required = ["SUPABASE_URL", "SUPABASE_ANON_KEY"]

[context]
decisions = ["iOS-native design system", "client-side economy enforcement (known debt)"]
next = ["validate draggable sheet on physical device"]

[agents]
installed = ["engineering-code-reviewer"]
```

A global cache is kept in your OS config directory to make `vibe list` instant.
It is **fully regenerable and never authoritative** — delete it and `vibe scan`
rebuilds it.

## Agents

`vibe agents` installs agent definitions — markdown files with frontmatter —
from a git-backed store into a project's `.claude/agents/`.

```console
$ vibe agents update                  # the only command that uses the network
$ vibe agents list                    # what the store offers
$ vibe agents add engineering-code-reviewer
$ vibe agents status                  # what this project has, and its state
$ vibe agents sync                    # after a fresh clone: install what's declared
```

Two files, and the split is the point. `.vibe/project.toml` declares *intent* —
a sorted list of names, committed, no hashes — so a teammate's `add` does not
turn the project's manifest into merge noise. `.vibe/agents.lock` records local
filesystem state: which files `vibe` wrote and what they hashed to.

The lockfile belongs in your `.gitignore`, and **`vibe` will not put it there** —
it does not write files you did not ask for, including helpful ones. The
generated header says so at the top of the file; adding the line is your call.

**`.claude/agents/` is not ours.** It holds hand-written agents and agents other
tools installed, so `vibe` only ever touches files it put there and can still
prove it wrote. An agent you edited is never overwritten without `--force`, an
agent deleted upstream is never deleted locally, and a file `vibe` did not
install is neither adopted nor removed.

`update` is the only command that touches the network. Everything else works
offline against whatever the store already holds — and says how old that is
rather than letting a stale store look like a complete one.

## Design rules

These are constraints, not aspirations. They are why the tool is shaped this way.

1. **Works 100% without an API key.** AI enrichment is strictly optional and
   additive. A tool whose adoption depends on an API key is a tool nobody adopts.
2. **Never destructive.** No command deletes your files. `archive` sets a flag.
   Every write can be previewed with `--dry-run`.
3. **Cross-platform.** Linux, macOS, and Windows are all first-class; the test
   suite runs on all three.
4. **Scan is fast.** 50 repositories in well under two seconds. Measured at a
   **654 ms median / 686 ms p90 warm, ~1067 ms median cold** on a 2017 quad-core
   with a SATA SSD, over a corpus of 50 single-commit git repositories carrying
   the usual `node_modules` / `target` / `.venv` noise. Cold costs ~1.5x rather
   than a multiple, because the scan barely touches the disk — 96% of the time
   is the two `git` subprocess calls per repository, and the same corpus with no
   `.git` at all indexes in 34 ms. Cost is linear at ~13 ms per project, so the
   two-second budget is exhausted at roughly **150 projects**. The harness,
   corpus definition and measurement protocol are in
   [`crates/vibe-core/examples/scan_bench.rs`](crates/vibe-core/examples/scan_bench.rs).
5. **Detection is honest.** When the stack cannot be inferred, the field is left
   empty and flagged. The tool does not guess, and it never invents a
   plausible-looking value.

## Architecture

```
crates/vibe-core   library — manifest types, detection, render engine.
                   No stdout, no clap. Reusable from a desktop frontend.
crates/vibectl     binary `vibe` — clap, terminal output, human-facing errors.
```

Git and GitHub work by shelling out to `git` and `gh`. There is no bundled
libgit2, no OAuth implementation, and no token storage — `gh` already owns auth.
If `gh` is missing, a `GITHUB_TOKEN` environment variable is used as a fallback;
if neither exists, the affected features degrade and the rest still works.

## Building

```console
$ cargo build --workspace
$ cargo test --workspace
```

Requires Rust 1.85 or newer (edition 2024).

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
