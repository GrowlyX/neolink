//! Create a virtual display and report whether WindowServer publishes it, plus
//! whether a rate change keeps the advertised list intact.
//!
//! `cargo run -p neolink-core --example modes_probe`

use neolink_core::displays;
use neolink_core::virtual_display::{VirtualDisplay, VirtualDisplayConfig, VirtualMode};

fn online(id: u32, ms: u64) -> bool {
    for _ in 0..ms / 100 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if displays::list().iter().any(|d| d.id == id) {
            return true;
        }
    }
    false
}

fn rates_at(id: u32, w: u32, h: u32) -> Vec<u32> {
    let mut v: Vec<u32> = displays::modes(id)
        .into_iter()
        .filter(|m| m.width == w && m.height == h)
        .map(|m| m.hz())
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn main() {
    let cfg = VirtualDisplayConfig {
        modes: vec![
            VirtualMode::with_refresh(1920, 1080, 60.0),
            VirtualMode::with_refresh(1920, 1080, 120.0),
        ],
        ..Default::default()
    };
    let vd = match VirtualDisplay::new(&cfg) {
        Ok(v) => v,
        Err(e) => {
            println!("could not create a display: {e}");
            return;
        }
    };
    let id = vd.display_id();
    if !online(id, 2500) {
        println!("display {id} was created but never came online");
        return;
    }
    println!("online, 1920x1080 rates {:?}", rates_at(id, 1920, 1080));

    for want in [120u32, 60] {
        let Some(mode) = displays::modes(id)
            .into_iter()
            .find(|m| m.width == 1920 && m.height == 1080 && m.hz() == want)
        else {
            println!("{want} Hz is not advertised");
            continue;
        };
        displays::apply(
            &[displays::Change {
                id,
                mode_id: Some(mode.id),
                ..Default::default()
            }],
            true,
        )
        .ok();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let kept = rates_at(id, 1920, 1080);
        let current = displays::list()
            .into_iter()
            .find(|d| d.id == id)
            .and_then(|d| d.mode)
            .map(|m| m.hz());
        println!("selected {want} Hz -> current {current:?}, rates still {kept:?}");
        assert_eq!(kept, [60, 120], "the advertised list shrank");
    }
}
