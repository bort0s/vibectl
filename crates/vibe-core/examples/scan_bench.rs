//! The 50-repo scan budget, defined before it is measured.
//!
//! An unqualified "under 2 seconds" is unfalsifiable, so this fixes all three
//! variables:
//!
//! **Corpus.** 50 project directories under one root, in a mix modelled on a
//! developer's `~/projects`: 20 Node, 10 Rust, 8 Python, 6 Go, 4 PHP, 2
//! polyglot. Each is a real git repository with one commit. Each carries the
//! noise directory its ecosystem actually produces — `node_modules`, `target`,
//! `.venv`, `vendor` — because pruning those is most of what makes a scan fast,
//! and a corpus without them measures nothing interesting.
//!
//! **Cache state.** Warm. The corpus is generated immediately before the run,
//! so it is in the OS page cache. This is the honest case to optimise for —
//! `vibe scan` is a command people re-run — and it is also the *faster* case,
//! so it must be reported as such rather than passed off as the general number.
//! A first-ever cold scan of a real `~/projects` will be slower and is not
//! measured here.
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

    // Three runs. The first is reported separately because it is the one a
    // user actually experiences after `cd`-ing somewhere new; the later ones
    // show whether anything is being cached that should not be.
    let mut timings = Vec::new();
    for run in 1..=3 {
        let started = Instant::now();
        let report = registry.scan(&req, &NullReporter);
        let elapsed = started.elapsed();
        timings.push(elapsed);
        println!(
            "run {run}: {:>7.1}ms   {} projects   {} suggestions   {} unreadable",
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

    let best = timings.iter().min().expect("three runs");
    let worst = timings.iter().max().expect("three runs");
    println!();
    println!(
        "best {:.1}ms / worst {:.1}ms for {projects} projects",
        best.as_secs_f64() * 1000.0,
        worst.as_secs_f64() * 1000.0
    );
    println!(
        "budget: 2000ms for 50 projects -> {}",
        if worst.as_millis() <= 2000 && projects >= 50 {
            "MET (worst case, warm cache)"
        } else if projects < 50 {
            "n/a (fewer than 50 projects)"
        } else {
            "MISSED"
        }
    );

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
