use neolink_core::{displays, layout, virtual_display::*};

fn main() {
    let cfg = VirtualDisplayConfig {
        modes: vec![VirtualMode::new(3840, 2160), VirtualMode::new(1920, 1080)],
        hidpi: true,
        ..Default::default()
    };
    let mut vd = VirtualDisplay::new(&cfg).unwrap();
    let id = vd.display_id();
    std::thread::sleep(std::time::Duration::from_millis(900));
    println!(
        "created: online={}",
        displays::list().iter().any(|d| d.id == id)
    );

    // shrink to a single mode (mirrors set_mode on a virtual display)
    let mut single = cfg.clone();
    single.modes = vec![VirtualMode::new(1920, 1080)];
    vd.reconfigure(&single).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    println!(
        "after shrink: online={}",
        displays::list().iter().any(|d| d.id == id)
    );

    // origin move, as the canvas does
    displays::apply(
        &[displays::Change {
            id,
            origin: Some((1600, 40)),
            ..Default::default()
        }],
        false,
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    println!(
        "after origin: online={}",
        displays::list().iter().any(|d| d.id == id)
    );

    // tidy(): layout::apply over all displays
    let all: Vec<layout::Entry> = displays::list()
        .iter()
        .map(|d| layout::Entry {
            uuid: d.uuid.clone(),
            name: d.name.clone(),
            x: d.x,
            y: d.y,
            mode_id: if d.id == id {
                None
            } else {
                d.mode.map(|m| m.id)
            },
            width: d.width,
            height: d.height,
            hidpi: false,
            rotation: 0,
            refresh: d.mode.map(|m| m.refresh),
            mirror_of: None,
            main: d.main,
        })
        .collect();
    let profile = layout::Profile {
        version: 1,
        displays: all,
    };
    layout::apply(&profile).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(700));
    println!(
        "after tidy: online={}",
        displays::list().iter().any(|d| d.id == id)
    );

    vd.reconfigure(&single).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(700));
    println!(
        "after reassert: online={}",
        displays::list().iter().any(|d| d.id == id)
    );
    if let Some(d) = displays::list().into_iter().find(|d| d.id == id) {
        println!("final: {}x{}", d.width, d.height);
    }
}
