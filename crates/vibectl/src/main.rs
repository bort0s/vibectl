//! `vibe` — command-line frontend for [`vibe_core`].
//!
//! This crate owns argument parsing, terminal rendering, and human-facing error
//! messages. All logic lives in `vibe-core`, and every byte printed here came
//! out of a core return value.

// In a binary crate nothing is reachable from outside, so `unreachable_pub`
// fires on every `pub` in a submodule. The lint earns its place in `vibe-core`,
// where it keeps the Tauri-facing API surface honest; here it is pure noise.
#![allow(unreachable_pub)]

mod cli;
mod exit;
mod output;
mod reporter;

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command, NewArgs};
use exit::Exit;
use vibe_core::{Config, NewRequest, Registry};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let code = match run(&cli) {
        Ok(code) => code,
        Err(err) => {
            let mut stderr = std::io::stderr();
            let _ = writeln!(stderr, "{}", output::error_lines(&err));
            Exit::Failure
        }
    };
    code.into()
}

fn run(cli: &Cli) -> Result<Exit, vibe_core::CoreError> {
    match &cli.command {
        Command::New(args) => cmd_new(args),
    }
}

fn cmd_new(args: &NewArgs) -> Result<Exit, vibe_core::CoreError> {
    let registry = Registry::open(Config::discover());

    let req = NewRequest::new(&args.name)
        .with_parent_dir(&args.path)
        .with_description(args.description.clone())
        .with_status(args.status.clone());

    let plan = registry.plan_new(&req)?;

    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

    if args.write.dry_run {
        if args.format.json {
            // The plan serialises directly. It is `Serialize` and deliberately
            // not `Deserialize`: this output is for humans and scripts to read,
            // never for anything to feed back into `apply`.
            let json = serde_json::to_string_pretty(&plan)
                .expect("WritePlan is a plain data structure and always serialises");
            let _ = writeln!(stdout, "{json}");
        } else {
            let _ = output::write_plan_human(&mut stdout, &plan);
            let _ = writeln!(stderr, "\ndry run — nothing was written");
        }
        return Ok(Exit::Success);
    }

    let rep = reporter::TermReporter::new(args.format.json);
    let report = registry.apply(&plan, &rep)?;

    if args.format.json {
        let json = serde_json::to_string_pretty(&report)
            .expect("ApplyReport is a plain data structure and always serialises");
        let _ = writeln!(stdout, "{json}");
    } else {
        let _ = writeln!(stdout, "Created {}", req.name());
        let _ = output::write_apply_human(&mut stdout, &report);
    }

    Ok(Exit::Success)
}
