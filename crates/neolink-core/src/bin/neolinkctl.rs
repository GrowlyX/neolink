//! Headless control surface: enumerate, arrange, and bring virtual displays online.
//!
//! `neolinkctl list` prints the desktop as CoreGraphics sees it, `neolinkctl layout` prints the
//! saved arrangement, and `neolinkctl add` creates a virtual display that lives until Ctrl-C.

use anyhow::Context;
use anyhow::Result;
use neolink_core::{
    agent, dfu, diff::DirtyTracker, displays, dl3, h264, layout, loginitem, spkg, transport,
    vendor, virtual_display,
};
use std::time::Duration;

/// Split `frame`'s arguments into positional values and the codec, which may
/// appear as `--codec X` or `--codec=X` anywhere.
fn frame_args() -> Result<(Vec<String>, h264::Codec)> {
    let mut positional = Vec::new();
    let mut codec = h264::Codec::Raw;
    let mut args = std::env::args().skip(2);
    while let Some(arg) = args.next() {
        if let Some(name) = arg.strip_prefix("--codec=") {
            codec =
                h264::Codec::parse(name).context(format!("unknown codec {name:?} (raw|h264)"))?;
        } else if arg == "--codec" {
            let name = args.next().context("--codec needs a value (raw|h264)")?;
            codec =
                h264::Codec::parse(&name).context(format!("unknown codec {name:?} (raw|h264)"))?;
        } else {
            positional.push(arg);
        }
    }
    Ok((positional, codec))
}

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
         \x20 neolinkctl hdcp              report protection state / HDCP 2.2\n\
         \x20 neolinkctl agent start|stop|restart\n\
         \x20 neolinkctl vendor status|cache|fetch|expand|install|provision|packs|purge\n\
         \x20 neolinkctl pack F            parse a vendor .spkg and list its frames\n\
         \x20 neolinkctl pack F --out D   write F's firmware image to D\n\
         \x20 neolinkctl flash F [--force]  DFU-flash a .spkg/.fullrom to the dock\n\
         \x20 neolinkctl frame [ID] [N] [--codec raw|h264]\n\
         \x20                           capture a display and ship N frames\n\
         \x20 neolinkctl login on|off|status\n\
         \x20 neolinkctl loginscreen on|off|status\n"
    );
    std::process::exit(2)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn progress(stage: vendor::Stage, done: u64, total: u64) {
    let label = match stage {
        vendor::Stage::Download => "download",
        vendor::Stage::Verify => "verify",
        vendor::Stage::Expand => "expand",
        vendor::Stage::Install => "install",
    };
    if total > 0 {
        eprint!(
            "\r{label}: {:.1} / {:.1} MiB",
            done as f64 / 1048576.0,
            total as f64 / 1048576.0
        );
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
            println!(
                "pin:              {} (sha256 {})",
                vendor::VERSION,
                &vendor::SHA256[..16]
            );
            println!(
                "installed app:    {}",
                s.installed_version.as_deref().unwrap_or("-")
            );
            println!("package cached:   {}", s.package_cached);
            println!("expanded payload: {}", s.expanded);
            println!(
                "cache size:       {:.1} MiB",
                s.cache_bytes as f64 / 1048576.0
            );
            println!("cache dir:        {}", vendor::paths()?.dir.display());
        }
        Some("cache") => println!("{}", vendor::paths()?.dir.display()),
        Some("fetch") => {
            let path = vendor::fetch(&mut progress)?;
            println!(
                "verified {} ({:.1} MiB)",
                path.display(),
                std::fs::metadata(&path)?.len() as f64 / 1048576.0
            )
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
            println!(
                "virtual display online: id={} (Ctrl-C to remove)",
                d.display_id()
            );
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
                    if agent::running() {
                        "(agent running)"
                    } else {
                        "(free)"
                    },
                    d.bus_id,
                    d.serial.as_deref().unwrap_or("-")
                );
            }
        }
        Some("edid") => {
            let mut t = transport::Transport::open()?;
            let edid = if std::env::args().any(|a| a == "--template") {
                dl3::template_edid(600, 340, "Neolink Display")
            } else if let Some(family) = std::env::args()
                .nth(3)
                .filter(|_| std::env::args().nth(2).as_deref() == Some("--dock"))
            {
                dl3::dock_edid(&family, 600, 340)
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
                    println!(
                        "{} bytes, valid EDID: {}",
                        edid.len(),
                        dl3::valid_edid(&edid)
                    );
                }
            }
        }
        Some("flash") => {
            let path = std::env::args()
                .nth(2)
                .context("flash <firmware.spkg|.fullrom> [--force]")?;
            let force = std::env::args().any(|a| a == "--force");
            let fw = dfu::Firmware::parse(&std::fs::read(&path)?)?;
            let dev = dfu::Device::open_matching(|_| true)?;
            let desc = dev.descriptor()?;
            println!("dock        {}", dev.desc.bus_id);
            println!(
                "firmware    customer/board {:#08x} id {:#018x}",
                desc.customer_board(),
                desc.firmware_id()
            );
            println!("image       {} bytes", fw.len());
            let functional = dev.functional();
            match &functional {
                Some(f) => println!("block size  {} (from descriptor)", f.block),
                None => println!(
                    "block size  {} (default, none readable)",
                    dfu::DEFAULT_BLOCK
                ),
            }
            if !force {
                let mut t = transport::Transport::open_matching(|d| d.bus_id == dev.desc.bus_id)?;
                let a = t.attributes()?;
                println!(
                    "protection  {:?} (hdcp {})",
                    a.protection,
                    if a.hdcp22 { "2.2" } else { "?" }
                );
            }
            let mut last = 0usize;
            let mut cb = |sent: usize, total: usize| {
                if sent == total || sent - last >= 512 * 1024 {
                    last = sent;
                    eprint!("\r{sent} / {total} bytes");
                    if sent == total {
                        eprintln!();
                    }
                }
            };
            let mut session = dev.session(functional.as_ref()).with_progress(&mut cb);
            let sent = session.flash(&fw)?;
            println!("Firmware sent ({sent} bytes)");
            println!("The dock re-enumerates; power-cycle it if it does not come back.");
        }
        Some("hdcp") => {
            let mut t = transport::Transport::open()?;
            let a = t.attributes()?;
            println!("dock        {}", t.desc.bus_id);
            println!("protection  {:?}", a.protection);
            println!(
                "hdcp        {}",
                match a.revision {
                    Some(r) => format!("{r:#04x}"),
                    None => "unknown".into(),
                }
            );
            println!("hdcp 2.2    {}", if a.hdcp22 { "yes" } else { "no" });
            println!("raw         {}", hex(&a.raw));
            for id in 0u8..20 {
                println!("  {id:2}  {}", transport::Transport::hdcp_name(id));
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
        Some("pack") => {
            let path = std::env::args()
                .nth(2)
                .context("pack <file.spkg> [--out DIR]")?;
            let k = spkg::parse_file(std::path::Path::new(&path))?;
            let f = k.frames()?;
            println!(
                "{} rev={:?} frames={} data={} trailing={}",
                k.name().unwrap_or_default(),
                k.text(spkg::Field::Revision),
                f.frames.len(),
                f.data().len(),
                f.rest
            );
            if let Some(dir) = std::env::args()
                .nth(4)
                .filter(|_| std::env::args().nth(3).as_deref() == Some("--out"))
            {
                let stem = std::path::Path::new(&path)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let out = std::path::Path::new(&dir).join(format!("{stem}.bin"));
                let image = f.data();
                std::fs::write(&out, &image)?;
                println!("wrote {} bytes to {}", image.len(), out.display());
            }
        }
        Some("frame") => {
            let (positional, codec) = frame_args()?;
            let want = positional.first().and_then(|a| a.parse::<u32>().ok());
            let count = positional
                .get(1)
                .and_then(|a| a.parse::<usize>().ok())
                .unwrap_or(1);
            let id = match want {
                Some(id) => id,
                None => displays::list()
                    .into_iter()
                    .find(|d| d.main)
                    .map(|d| d.id)
                    .context("no main display")?,
            };
            let mut t = transport::Transport::open()?;
            t.set_codec(codec);
            println!(
                "dock {} max payload {} bytes, codec {}",
                t.desc.bus_id,
                t.max_payload,
                codec.name()
            );
            let mut tracker: Option<DirtyTracker> = None;
            let mut full_bytes = 0usize;
            let mut diff_bytes = 0usize;
            for i in 0..count {
                let g = displays::grab(id).context("display returned no image")?;
                let started = std::time::Instant::now();
                let stale = tracker.as_ref().is_none_or(|tr| {
                    tr.width() != g.width as u32 || tr.height() != g.height as u32
                });
                if stale {
                    tracker = Some(DirtyTracker::new(g.width as u32, g.height as u32));
                }
                let tr = tracker.as_mut().expect("tracker was just created");
                let (damage, sent) = t.frame_dirty(tr, g.stride, &g.pixels)?;
                let raw = g.stride * g.height;
                full_bytes += raw;
                diff_bytes += sent;
                println!(
                    "frame {i}: {}x{} {} of {} tiles dirty -> {} rects -> {} of {} bytes USB in {:?}",
                    g.width,
                    g.height,
                    damage.dirty_tiles,
                    tr.width().div_ceil(16) * tr.height().div_ceil(16),
                    damage.rects.len(),
                    sent,
                    raw,
                    started.elapsed()
                );
            }
            if count > 1 && full_bytes > 0 {
                println!(
                    "{} frames: {} bytes raw -> {} bytes on the wire ({:.1}% saved)",
                    count,
                    full_bytes,
                    diff_bytes,
                    100.0 * (1.0 - diff_bytes as f64 / full_bytes as f64)
                );
            }
        }
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
                loginitem::bundle()
                    .map(|p| format!(" ({})", p.display()))
                    .unwrap_or_default()
            ),
        },
        Some("loginscreen") => match std::env::args().nth(2).as_deref() {
            Some("on") => {
                loginitem::install_login_screen()?;
                println!("enabled at the login window");
            }
            Some("off") => {
                loginitem::remove_login_screen()?;
                println!("disabled at the login window");
            }
            _ => println!("login window agent: {}", loginitem::login_screen_enabled()),
        },
        Some("probe") => {
            println!(
                "CGVirtualDisplay available: {}",
                virtual_display::is_available()
            );
            println!(
                "screen recording allowed:  {}",
                displays::screen_capture_allowed()
            );
            println!(
                "vendor agent installed:    {}",
                neolink_core::agent::installed()
            );
            println!(
                "vendor XPC reachable:      {}",
                neolink_core::agent::reachable()
            );
        }
        Some(_) => usage(),
    }
    Ok(())
}
