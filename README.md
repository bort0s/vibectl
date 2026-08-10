# vibectl

**A registry for the half-finished projects you already have.**

You have 30 project directories. You remember what maybe six of them do. `vibe`
indexes the rest — inferring stack, git state, and deploy target from what is
already on disk — and keeps a small manifest next to each one so you (and your
agents) can pick any of them back up without archaeology.

> **Status: pre-alpha.** The workspace skeleton and CI are in place. Nothing is
> implemented yet and nothing is published to crates.io. Do not install this.

## The idea

Most project tooling assumes you are starting from zero. This one assumes you
are starting from a mess.

```console
$ vibe scan ~/projects
  indexed 34 projects in 1.2s · 28 with git remotes · 6 undetectable stacks

$ vibe list
  NAME         STACK              STATUS   LAST COMMIT   DEPLOY
  macroring    node@22 · react    active    2 days ago   vercel
  tideline     rust@1.97          paused    7 months ago —
  otterbase    python@3.12        idea      —            —
```

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
```

A global cache is kept in your OS config directory to make `vibe list` instant.
It is **fully regenerable and never authoritative** — delete it and `vibe scan`
rebuilds it.

## Design rules

These are constraints, not aspirations. They are why the tool is shaped this way.

1. **Works 100% without an API key.** AI enrichment is strictly optional and
   additive. A tool whose adoption depends on an API key is a tool nobody adopts.
2. **Never destructive.** No command deletes your files. `archive` sets a flag.
   Every write can be previewed with `--dry-run`.
3. **Cross-platform.** Linux, macOS, and Windows are all first-class; the test
   suite runs on all three.
4. **Scan is fast.** 50 repositories in under two seconds. `node_modules`,
   `target`, `.git`, `dist`, and `vendor` are never descended into.
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
