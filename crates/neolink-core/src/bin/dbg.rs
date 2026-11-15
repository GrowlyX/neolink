use neolink_core::displays;
use neolink_core::virtual_display::*;

fn show(tag: &str, id: u32) {
    let d = displays::list().into_iter().find(|d| d.id == id);
    match d {
        Some(d) => println!(
            "{tag:<26} {}x{} px{}x{} rot{}",
            d.width,
            d.height,
            d.mode.map(|m| m.pixel_width).unwrap_or(0),
            d.mode.map(|m| m.pixel_height).unwrap_or(0),
            d.rotation
        ),
        None => println!("{tag:<26} MISSING"),
    }
}

fn main() {
    let base = VirtualDisplayConfig {
        modes: vec![VirtualMode::new(1920, 1080)],
        hidpi: true,
        ..Default::default()
    };
    let mut vd = VirtualDisplay::new(&base).unwrap();
    let id = vd.display_id();
    std::thread::sleep(std::time::Duration::from_millis(700));
    show("created(hidpi)", id);

    let mut c = base.clone();
    c.rotation = 90;
    vd.reconfigure(&c).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(700));
    show("rot90", id);

    c.rotation = 0;
    vd.reconfigure(&c).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(700));
    show("rot0 again", id);

    displays::apply(&[displays::Change { id, origin: Some((1600, 40)), ..Default::default() }], false).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(700));
    show("after origin", id);

    vd.reconfigure(&c).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(700));
    show("reassert", id);
}
