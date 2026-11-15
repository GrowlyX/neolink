//! Print or write a synthesised EDID block: `edidgen Wmm Hmm NAME [out.bin]`.
fn main() {
    let mut a = std::env::args().skip(1);
    let w: u16 = a.next().and_then(|s| s.parse().ok()).unwrap_or(600);
    let h: u16 = a.next().and_then(|s| s.parse().ok()).unwrap_or(340);
    let name = a.next().unwrap_or_else(|| "Neolink Display".into());
    let e = neolink_core::dl3::template_edid(w, h, &name);
    match a.next() {
        Some(path) => {
            std::fs::write(&path, &e).unwrap();
            println!("wrote {path}");
        }
        None => {
            for (i, c) in e.chunks(16).enumerate() {
                print!("{i:04x} ");
                for b in c {
                    print!("{b:02x} ");
                }
                println!();
            }
        }
    }
}
