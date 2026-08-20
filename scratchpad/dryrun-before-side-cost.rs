//! What the dry run's before-side read costs, measured rather than assumed.
//!
//! This measures the two operations `TargetNow::Occupied` performs --
//! `read_to_string` on the target, then `lines().count()` and a line-by-line
//! write of the result -- across sizes spanning the real callers and well past
//! them. It is NOT an end-to-end render: `write_plan_human` lives in a binary
//! crate and cannot be linked from here, so what is measured is the mechanism,
//! and the difference is declared rather than glossed.
//!
//! Build:  rustc -O ceiling.rs -o ceiling && ./ceiling

use std::io::Write;
use std::time::Instant;

fn main() {
    let dir = std::env::temp_dir().join("vibe-ceiling-measure");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // One 80-column line, the shape of every file these callers actually write.
    let line = "x".repeat(79);
    println!(
        "{:>12}  {:>10}  {:>9}  {:>9}  {:>9}",
        "bytes", "lines", "read ms", "count ms", "render ms"
    );

    for &kb in &[4usize, 32, 512, 8 * 1024, 128 * 1024] {
        let path = dir.join(format!("t{kb}.txt"));
        {
            let f = std::fs::File::create(&path).unwrap();
            let mut w = std::io::BufWriter::new(f);
            let mut written = 0usize;
            while written < kb * 1024 {
                writeln!(w, "{line}").unwrap();
                written += 80;
            }
        }
        let bytes = std::fs::metadata(&path).unwrap().len();

        let t = Instant::now();
        let before = std::fs::read_to_string(&path).unwrap();
        let read_ms = t.elapsed().as_secs_f64() * 1000.0;

        let t = Instant::now();
        let n = before.lines().count();
        let count_ms = t.elapsed().as_secs_f64() * 1000.0;

        // What the renderer then does with it: one write per line, to a sink
        // that is a terminal in practice. Rendered to a Vec so the measurement
        // is of the loop and not of the console.
        let t = Instant::now();
        let mut out: Vec<u8> = Vec::new();
        for l in before.lines() {
            writeln!(out, "  - {l}").unwrap();
        }
        let render_ms = t.elapsed().as_secs_f64() * 1000.0;

        println!("{bytes:>12}  {n:>10}  {read_ms:>9.1}  {count_ms:>9.1}  {render_ms:>9.1}");
        drop(out);
        std::fs::remove_file(&path).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
