//! Writes every logo file from the Rust geometry.
//!
//!     cargo run --example export_svg -- out/
//!
//! Add `--cubics` to write arc-free paths (what a path builder without an arc
//! command would draw).

use neolink_brand::logo::{lockup, mark, wordmark};
use std::{env, fs, path::PathBuf};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let cubics = args.iter().any(|a| a == "--cubics");
    let dir = PathBuf::from(
        args.iter()
            .find(|a| !a.starts_with("--"))
            .map_or("out", |s| s),
    );
    fs::create_dir_all(&dir)?;

    let prep = |s: neolink_brand::logo::Shape| if cubics { s.to_cubics() } else { s };
    let (mark, wordmark, lockup) = (prep(mark()), prep(wordmark()), prep(lockup()));

    let files = [
        ("neolink-mark-black.svg", mark.to_svg("#000", None, 0.0)),
        ("neolink-mark-white.svg", mark.to_svg("#fff", None, 0.0)),
        (
            "neolink-wordmark-black.svg",
            wordmark.to_svg("#000", None, 0.0),
        ),
        (
            "neolink-wordmark-white.svg",
            wordmark.to_svg("#fff", None, 0.0),
        ),
        ("neolink-lockup-black.svg", lockup.to_svg("#000", None, 0.0)),
        ("neolink-lockup-white.svg", lockup.to_svg("#fff", None, 0.0)),
        ("neolink-icon.svg", mark.to_svg("#fff", Some("#000"), 50.0)),
        (
            "neolink-icon-inverse.svg",
            mark.to_svg("#000", Some("#fff"), 50.0),
        ),
    ];
    for (name, svg) in &files {
        fs::write(dir.join(name), svg)?;
    }
    println!("wrote {} files to {}", files.len(), dir.display());
    Ok(())
}
