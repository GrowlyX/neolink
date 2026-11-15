use neolink_core::virtual_display::*;
fn main() {
    let mut cfg = VirtualDisplayConfig {
        modes: vec![VirtualMode::new(1920, 1080), VirtualMode::new(1280, 720)],
        ..Default::default()
    };
    let mut d = VirtualDisplay::new(&cfg).unwrap();
    let id = d.display_id();
    println!("id={id}");
    std::thread::sleep(std::time::Duration::from_millis(700));
    println!(
        "before: {:?}",
        neolink_core::displays::list()
            .iter()
            .find(|x| x.id == id)
            .map(|x| (x.width, x.height))
    );
    cfg.rotation = 90;
    cfg.hidpi = false;
    println!(
        "reconfig -> {:?}",
        d.reconfigure(&cfg).map_err(|e| e.to_string())
    );
    std::thread::sleep(std::time::Duration::from_millis(900));
    println!(
        "after: {:?}",
        neolink_core::displays::list()
            .iter()
            .find(|x| x.id == id)
            .map(|x| (x.width, x.height))
    );
}
