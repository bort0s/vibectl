//! The 50-repo scan budget, defined before it is measured.
//!
//! An unqualified "under 2 seconds" is unfalsifiable, so this fixes all three
//! variables:
//!
//! **Corpus.** 50 project directories under one root, in a mix modelled on a
//! developer's `~/projects`: 20 Node, 10 Rust, 8 Python, 6 Go, 4 PHP, 2
//! polyglot. Each is a real git repository with one commit. Each carries the
//! noise directory its ecosystem actually produces — `node_modules`, `target`,
//! `.venv`, `vendor`.
//!
//! **Correction, recorded so nobody reads the corpus size as evidence:** the
//! noise volumes measure nothing. Stripping all 20,478 noise files — leaving 22
//! files per project — changed the median by less than the run-to-run spread.
//! The prune set works well enough that walking is invisible beside spawning
//! `git`: with no `.git` present at all, 50 projects index in **34 ms**. The
//! noise is kept because a corpus without it would be unrepresentative, not
//! because it is load-bearing for the number.
//!
//! **The practical ceiling is ~150 projects.** Cost is ~13.2 ms/project and
//! stays linear to at least 500, because the bottleneck is a fixed per-repo
//! subprocess cost rather than anything that grows with N. If a user ever
//! reports a slow scan, project count is the first number to check.
//!
//! **Cache state.** Warm, and **cold is UNMEASURED**. The corpus is generated
//! immediately before the run, so it is in the OS page cache. Warm is the
//! honest case to optimise for — `vibe scan` is a command people re-run — but
//! it is also the faster one, and dropping the page cache on Windows needs
//! administrator rights this was not run with. A first-ever cold scan will be
//! slower by an unknown amount. Do not let the warm number stand in for both.
//!
//! **Protocol.** Three runs, min/max, is retired — it does not survive contact
//! with this machine. The same binary and corpus produced a 583 ms median in
//! one sitting and a 684 ms median in another, differing only in what else was
//! running, with the spread widening 3.6x. Within-sitting variance is small;
//! *between*-sitting drift is several times larger, and n=3 samples one cluster
//! and reports it as the number.
//!
//! An acceptable protocol for any future claim:
//!
//! - At least 11 runs in one sitting, reporting **median and p90**, never a
//!   single best case.
//! - Repeat in a second sitting and report both medians. A claim that does not
//!   survive the gap between sittings is noise.
//!   `--runs N` sets the sample size.
//! - State the machine, the cache state, and the corpus alongside the number,
//!   every time. A number without those three is not comparable to anything.
//! - **Check the antivirus.** On Windows this is the single largest
//!   confounder, and it is invisible unless looked for. Three measurements of
//!   this same corpus and binary in one session gave medians of 583ms, 684ms
//!   and 1012ms, with the standard deviation going from 15 to 142 — the last
//!   one taken while Defender real-time protection was indexing the ~250k
//!   files the benchmark had just created. Generating a large corpus and
//!   immediately measuring against it means measuring the antivirus.
//!   `Get-MpComputerStatus` reports whether a scan is in flight.
//!
//! **Caveat on the reported figure.** The ~900ms median quoted for P2 predates
//! the antivirus check. It stands as reported, but it was taken without
//! confirming Defender was idle, and the corpus had just been generated — so it
//! is more likely an over-estimate than an under-estimate. Any figure quoted
//! from here on must state that the check was made.
//!
//! **Cold cache, measured after all.** A standby-list eviction harness (no
//! admin rights needed: allocate and touch N GiB to force a trim) put cold at a
//! ~1067ms median against ~684ms warm — **~1.5x, not a multiple** — because the
//! scan barely reads from disk. Corpus metadata costs ~35ms cold and `git.exe`
//! plus its DLLs ~80ms *once*, not per call. A 2.8s outlier reproduced twice is
//! whole-system first-run-after-boot image faulting, not scan-specific.
//! Eviction decays: only the first trial after a busy period is genuinely cold,
//! so trust trial 1 and discard the rest.
//!
//! **Shapes this corpus still omits**, each of which a real `~/projects` has:
//! a repository with >1000 refs (the one shape measured to break the budget), a
//! git worktree, a `.gitignore` in every project (~+40% walk cost), and
//! realistic source-file volume. Walk cost is ~5% of the total, so the last two
//! move the number by ~2% — but the first is not a rounding error.
//!
//! CI cannot keep a wall-clock budget at all — a shared runner's drift exceeds
//! what is being measured. `tests/scan_budget.rs` guards the *cause* instead:
//! at most two `git` invocations per repository, which is ~96% of scan time and
//! is deterministic.
//!
//! **Hardware.** Recorded by the caller, not by this program. Numbers without a
//! machine attached are not comparable.
//!
//! Run with:
//!
//! ```text
//! cargo run --release --example scan_bench -- [--projects N] [--no-git] [--keep]
//! ```

use std::path::Path;
use std::time::Instant;

use vibe_core::{Config, NoRunner, NullReporter, Registry, ScanRequest};

struct Shape {
    kind: &'static str,
    count: usize,
    /// Files inside the ecosystem's noise directory, which must be pruned.
    noise_dir: &'static str,
    noise_files: usize,
    source_files: usize,
}

const SHAPES: &[Shape] = &[
    Shape {
        kind: "node",
        count: 20,
        noise_dir: "node_modules",
        noise_files: 500,
        source_files: 25,
    },
    Shape {
        kind: "rust",
        count: 10,
        noise_dir: "target",
        noise_files: 400,
        source_files: 20,
    },
    Shape {
        kind: "python",
        count: 8,
        noise_dir: ".venv",
        noise_files: 300,
        source_files: 20,
    },
    Shape {
        kind: "go",
        count: 6,
        noise_dir: "vendor",
        noise_files: 200,
        source_files: 20,
    },
    Shape {
        kind: "php",
        count: 4,
        noise_dir: "vendor",
        noise_files: 300,
        source_files: 20,
    },
    Shape {
        kind: "polyglot",
        count: 2,
        noise_dir: "node_modules",
        noise_files: 200,
        source_files: 20,
    },
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let use_git = !args.iter().any(|a| a == "--no-git");
    let keep = args.iter().any(|a| a == "--keep");
    let scale: usize = args
        .iter()
        .position(|a| a == "--projects")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let runs: usize = args
        .iter()
        .position(|a| a == "--runs")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(11);

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();

    println!("corpus root: {}", root.display());
    let t0 = Instant::now();
    let (projects, files) = build_corpus(&root, scale, use_git);
    println!(
        "generated {projects} projects / {files} files in {:.1}s (setup, not measured)",
        t0.elapsed().as_secs_f64()
    );
    println!("git repositories: {}", if use_git { "yes" } else { "no" });
    println!();

    let registry = if use_git {
        Registry::open(Config::default())
    } else {
        Registry::open(Config::default()).with_runner(std::sync::Arc::new(NoRunner))
    };
    let req = ScanRequest::new(&root);

    let mut timings = Vec::new();
    for run in 1..=runs {
        let started = Instant::now();
        let report = registry.scan(&req, &NullReporter);
        let elapsed = started.elapsed();
        timings.push(elapsed);
        println!(
            "run {run:>3}: {:>7.1}ms   {} projects   {} suggestions   {} unreadable",
            elapsed.as_secs_f64() * 1000.0,
            report.projects.len(),
            report.suggestion_count(),
            report.unreadable(),
        );
        assert_eq!(
            report.projects.len(),
            projects,
            "the scan must find every project it was given"
        );
    }

    // Median and p90, not best-of-N. A best case is the one number guaranteed
    // not to describe what a user experiences.
    let mut sorted: Vec<f64> = timings.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let p90 = sorted[((sorted.len() * 9) / 10).min(sorted.len() - 1)];
    let min = sorted[0];
    let max = sorted[sorted.len() - 1];

    println!();
    println!(
        "n={} projects={projects}  min {min:.0}  med {median:.0}  p90 {p90:.0}  max {max:.0} (ms)",
        sorted.len()
    );
    println!(
        "budget 2000ms/50 projects -> {}   [warm cache; cold UNMEASURED]",
        if p90 <= 2000.0 && projects >= 50 {
            "MET at p90"
        } else if projects < 50 {
            "n/a (fewer than 50 projects)"
        } else {
            "MISSED"
        }
    );
    println!("repeat in a second sitting and compare medians before believing this");

    if keep {
        let kept = std::env::temp_dir().join("vibe-bench-corpus");
        let _ = std::fs::rename(&root, &kept);
        println!("corpus kept at {}", kept.display());
        std::mem::forget(tmp);
    }
}

fn build_corpus(root: &Path, scale: usize, use_git: bool) -> (usize, usize) {
    let total: usize = SHAPES.iter().map(|s| s.count).sum();
    let mut made = 0;
    let mut files = 0;

    for shape in SHAPES {
        // Scale proportionally when asked for a different project count.
        let count = (shape.count * scale).div_ceil(total);
        for i in 0..count {
            if made >= scale {
                break;
            }
            let dir = root.join(format!("{}-{i:02}", shape.kind));
            files += build_project(&dir, shape, use_git);
            made += 1;
        }
    }
    (made, files)
}

fn build_project(dir: &Path, shape: &Shape, use_git: bool) -> usize {
    std::fs::create_dir_all(dir).expect("mkdir");
    let mut files = 0;

    let manifests: &[(&str, &str)] = match shape.kind {
        "node" => &[
            ("package.json", NODE_PKG),
            ("package-lock.json", "{\"lockfileVersion\":3}"),
            (".env.example", "DATABASE_URL=\nSUPABASE_ANON_KEY=\n"),
            ("vercel.json", "{\"framework\":\"nextjs\"}"),
        ],
        "rust" => &[("Cargo.toml", CARGO), ("Cargo.lock", "version = 4")],
        "python" => &[
            ("pyproject.toml", PYPROJECT),
            ("requirements.txt", "fastapi\npydantic\n"),
        ],
        "go" => &[("go.mod", GOMOD), ("go.sum", "")],
        "php" => &[("composer.json", COMPOSER)],
        _ => &[("package.json", NODE_PKG), ("go.mod", GOMOD)],
    };
    for (name, body) in manifests {
        std::fs::write(dir.join(name), body).expect("write");
        files += 1;
    }
    std::fs::write(dir.join("README.md"), "# project\n").expect("write");
    files += 1;

    let src = dir.join("src");
    std::fs::create_dir_all(&src).expect("mkdir");
    for i in 0..shape.source_files {
        std::fs::write(src.join(format!("mod{i}.txt")), "// source\n").expect("write");
        files += 1;
    }

    // The noise the prune set exists for. Spread across subdirectories, the
    // way a real dependency tree is, so pruning has to happen at the top of
    // the subtree rather than per file.
    let noise = dir.join(shape.noise_dir);
    for i in 0..shape.noise_files {
        let pkg = noise.join(format!("pkg{}", i % 50));
        std::fs::create_dir_all(&pkg).expect("mkdir");
        std::fs::write(pkg.join(format!("f{i}.js")), "module.exports={}\n").expect("write");
        files += 1;
    }

    if use_git {
        run_git(dir, &["init", "-q"]);
        run_git(dir, &["config", "user.email", "bench@example.invalid"]);
        run_git(dir, &["config", "user.name", "bench"]);
        run_git(dir, &["config", "commit.gpgsign", "false"]);
        run_git(
            dir,
            &["remote", "add", "origin", "git@github.com:bort0s/bench.git"],
        );
        // Only the manifests, so `git add` does not walk the noise directory.
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-q", "-m", "initial", "--no-verify"]);
    }
    files
}

fn run_git(dir: &Path, args: &[&str]) {
    let _ = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

const NODE_PKG: &str = r#"{
  "name": "bench",
  "description": "a benchmark fixture",
  "engines": { "node": ">=22" },
  "dependencies": {
    "react": "^19.0.0", "next": "^15.0.0", "vite": "^5.0.0",
    "@supabase/supabase-js": "^2.0.0", "stripe": "^17.0.0"
  },
  "devDependencies": { "typescript": "^5.0.0", "tailwindcss": "^3.0.0" }
}"#;

const CARGO: &str = r#"[package]
name = "bench"
description = "a benchmark fixture"
rust-version = "1.85"

[dependencies]
tokio = "1"
axum = "0.7"
serde = "1"
"#;

const PYPROJECT: &str = r#"[project]
name = "bench"
description = "a benchmark fixture"
requires-python = ">=3.12"
dependencies = ["fastapi", "pydantic", "sqlalchemy"]
"#;

const GOMOD: &str = r#"module example.com/bench

go 1.23

require (
	github.com/gin-gonic/gin v1.10.0
	gorm.io/gorm v1.25.0
)
"#;

const COMPOSER: &str = r#"{
  "description": "a benchmark fixture",
  "require": { "php": ">=8.2", "laravel/framework": "^11.0" }
}"#;
