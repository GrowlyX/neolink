fn main() {
    for d in neolink_core::displays::list() {
        println!("{} id={} {}x{} mode={:?}", d.name, d.id, d.width, d.height, d.mode);
        for m in neolink_core::displays::modes(d.id) {
            if m.width == 800 || m.id == 0 { println!("   * {} id={} {}x{}", m.label(), m.id, m.pixel_width, m.pixel_height); }
        }
    }
}
