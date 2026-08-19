//! One atomic replacement, so an external harness can ask whether it succeeded.
//!
//! Used by `scratchpad/rename-over-open.js` to measure whether a rename can
//! replace a destination another process holds open — the case ADR-0011 §7b
//! meets as the normal one, since install runs while Claude Code runs.
fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(contents)) = (args.next(), args.next()) else {
        eprintln!("usage: atomic_replace_once <path> <contents>");
        std::process::exit(2);
    };
    match vibe_core::write_atomically(std::path::Path::new(&path), &format!("{contents}\n")) {
        Ok(()) => println!("ok"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
