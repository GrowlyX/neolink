//! Headless control surface: enumerate, arrange, and bring virtual displays online.
//!
//! `neolinkctl list` prints the desktop as CoreGraphics sees it, `neolinkctl layout` prints the
//! saved arrangement, and `neolinkctl add` creates a virtual display that lives until Ctrl-C.

use anyhow::Result;
use neolink_core::{agent, displays, dl3, layout, loginitem, transport, vendor, virtual_display};
use std::time::Duration;

fn usage() -> ! {
    eprintln!(
        "neolinkctl — Neolink display control\n\n\
         USAGE:\n  neolinkctl list              enumerate displays and modes\n\
         \x20 neolinkctl layout            print the saved arrangement profile\n\
         \x20 neolinkctl apply             re-apply the saved arrangement\n\
         \x20 neolinkctl add [WxH]         create a virtual display and hold it\n\
         \x20 neolinkctl probe             report private-API availability\n\
         \x20 neolinkctl usb               list DisplayLink adapters\n\
         \x20 neolinkctl edid [--out F] [--template]\n\
         \x20 neolinkctl edid-write F     write an EDID block to the dock\n\
         \x20 neolinkctl agent start|stop|restart\n\
         \x20 neolinkctl vendor status|cache|fetch|expand|install|provision|packs|purge\n\
         \x20 neolinkctl login on|off|status\n"
    );
    std::process::exit(2)
}

fn progress(stage: vendor::Stage, done: u64, total: u64) {
    let label = match stage {
        vendor::Stage::Download => "download",
        vendor::Stage::Verify => "verify",
        vendor::Stage::Expand => "expand",
        vendor::Stage::Install => "install",
    };
    if total > 0 {
        eprint!("\r{label}: {:.1} / {:.1} MiB", done as f64 / 1048576.0, total as f64 / 1048576.0);
    } else {
        eprint!("\r{label}: {:.1} MiB", done as f64 / 1048576.0);
    }
    if done == total || stage != vendor::Stage::Download {
        eprintln!();
    }
}

/// Package handling. Nothing here ships in the repo: every byte comes from Synaptics.
fn vendor_cmd(action: Option<&str>) -> Result<()> {
    match action {
        Some("status") | None => {
            let s = vendor::status();
            println!("pin:              {} (sha256 {})", vendor::VERSION, &vendor::SHA256[..16]);
            println!("installed app:    {}", s.installed_version.as_deref().unwrap_or("-"));
            println!("package cached:   {}", s.package_cached);
            println!("expanded payload: {}", s.expanded);
            println!("cache size:       {:.1} MiB", s.cache_bytes as f64 / 1048576.0);
            println!("cache dir:        {}", vendor::paths()?.dir.display());
        }
        Some("cache") => println!("{}", vendor::paths()?.dir.display()),
        Some("fetch") => {
            let path = vendor::fetch(&mut progress)?;
            println!("verified {} ({:.1} MiB)", path.display(), std::fs::metadata(&path)?.len() as f64 / 1048576.0)
        }
        Some("expand") => println!("{}", vendor::expand(&mut progress)?.display()),
        Some("install") => {
            vendor::install()?;
            println!("installed DisplayLink Manager {}", vendor::VERSION);
        }
        Some("provision") => {
            vendor::provision(&mut progress)?;
            println!("DisplayLink Manager {} installed", vendor::VERSION);
        }
        Some("packs") => {
            for p in vendor::resource_packs()? {
                println!("{}", p.display());
            }
        }
        Some("purge") => println!("freed {:.1} MiB", vendor::purge()? as f64 / 1048576.0),
        Some(_) => usage(),
    }
    Ok(())
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
        Some("usb") => {
            let found = transport::devices()?;
            if found.is_empty() {
                println!("no DisplayLink adapter attached");
            }
            for d in found {
                println!(
                    "{:04x}:{:04x} {} bus {} serial {}",
                    d.vendor_id,
                    d.product_id,
                    if agent::running() { "(agent running)" } else { "(free)" },
                    d.bus_id,
                    d.serial.as_deref().unwrap_or("-")
                );
            }
        }
        Some("edid") => {
            let mut t = transport::Transport::open()?;
            let edid = if std::env::args().any(|a| a == "--template") {
                dl3::template_edid(600, 340, "Neolink Display")
            } else {
                t.edid_or_template(600, 340, "Neolink Display")
            };
            match std::env::args().nth(3) {
                Some(path) if std::env::args().nth(2).as_deref() == Some("--out") => {
                    std::fs::write(&path, &edid)?;
                    println!("wrote {} bytes to {path}", edid.len());
                }
                _ => {
                    for (i, chunk) in edid.chunks(16).enumerate() {
                        print!("{i:04x} ");
                        for b in chunk {
                            print!("{b:02x} ");
                        }
                        println!();
                    }
                    println!("{} bytes, valid EDID: {}", edid.len(), dl3::valid_edid(&edid));
                }
            }
        }
        Some("edid-write") => {
            let path = std::env::args().nth(2).unwrap_or_else(|| usage());
            let edid = std::fs::read(&path)?;
            let mut t = transport::Transport::open()?;
            t.write_edid(&edid)?;
            println!("wrote {} bytes of EDID", edid.len());
        }
        Some("agent") => match std::env::args().nth(2).as_deref() {
            Some("start") => {
                agent::start()?;
                println!("agent started");
            }
            Some("stop") => {
                agent::stop()?;
                println!("agent stopped");
            }
            Some("restart") => {
                agent::restart()?;
                println!("agent restarted");
            }
            _ => usage(),
        },
        Some("vendor") => vendor_cmd(std::env::args().nth(2).as_deref())?,
        Some("login") => match std::env::args().nth(2).as_deref() {
            Some("on") => {
                loginitem::install()?;
                println!("enabled at login");
            }
            Some("off") => {
                loginitem::remove()?;
                println!("disabled at login");
            }
            _ => println!(
                "at login: {}{}",
                loginitem::enabled(),
                loginitem::bundle().map(|p| format!(" ({})", p.display())).unwrap_or_default()
            ),
        },
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
