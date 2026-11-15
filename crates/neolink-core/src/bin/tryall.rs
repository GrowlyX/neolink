fn main() {
    let p = std::env::args().nth(1).unwrap();
    let b = std::fs::read(&p).unwrap();
    let k = match neolink_core::spkg::parse_file(std::path::Path::new(&p)) { Ok(k) => k, Err(e) => { println!("{e}"); return } };
    let pl = k.payload();
    println!("payload {}", pl.len());
    for skip in 0..12 {
        if skip >= pl.len() { break }
        match neolink_core::spkg::inflate(&pl[skip..]) {
            Ok(o) => println!("  skip {skip} -> {} bytes, head {:02x?}", o.len(), &o[..8.min(o.len())]),
            Err(_) => {}
        }
    }
}
