//! Headless control surface: enumerate, arrange, and bring virtual displays online.
//!
//! `neolinkctl list` prints the desktop as CoreGraphics sees it, `neolinkctl layout` prints the
//! saved arrangement, and `neolinkctl add` creates a virtual display that lives until Ctrl-C.

use anyhow::Result;
use neolink_core::{displays, layout, virtual_display};
use std::time::Duration;

fn usage() -> ! {
    eprintln!(
        "neolinkctl — Neolink display control\n\n\
         USAGE:\n  neolinkctl list              enumerate displays and modes\n\
         \x20 neolinkctl layout            print the saved arrangement profile\n\
         \x20 neolinkctl apply             re-apply the saved arrangement\n\
         \x20 neolinkctl add [WxH]         create a virtual display and hold it\n\
         \x20 neolinkctl probe             report private-API availability\n"
    );
    std::process::exit(2)
}

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("list") | None => {
            for d in displays::list() {
                println!(
                    "{:<28} id={:<3} {}×{} at ({}, {}){}",
                    d.name,
                    d.id,
                    d.width,
                    d.height,
                    d.x,
                    d.y,
                    if d.main { "  [main]" } else { "" }
                );
                for m in displays::modes(d.id).iter().take(8) {
                    println!("    {}", m.label());
                }
            }
        }
        Some("layout") => println!("{}", serde_json::to_string_pretty(&layout::load()?)?),
        Some("apply") => {
            let profile = layout::load()?;
            layout::apply(&profile)?;
            println!("arrangement applied");
        }
        Some("add") => {
            let (w, h) = match std::env::args().nth(2) {
                Some(spec) => {
                    let (w, h) = spec.split_once('x').unwrap_or(("1920", "1080"));
                    (w.parse().unwrap_or(1920), h.parse().unwrap_or(1080))
                }
                None => (1920, 1080),
            };
            let cfg = virtual_display::VirtualDisplayConfig {
                modes: vec![virtual_display::VirtualMode::new(w, h)],
                ..Default::default()
            };
            let d = virtual_display::VirtualDisplay::new(&cfg)?;
            println!("virtual display online: id={} (Ctrl-C to remove)", d.display_id());
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        Some("probe") => {
            println!("CGVirtualDisplay available: {}", virtual_display::is_available());
            println!("screen recording allowed:  {}", displays::screen_capture_allowed());
            println!("vendor agent installed:    {}", neolink_core::agent::installed());
            println!("vendor XPC reachable:      {}", neolink_core::agent::reachable());
        }
        Some(_) => usage(),
    }
    Ok(())
}
