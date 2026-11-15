fn main() {
    let p = std::env::args().nth(1).unwrap();
    let k = neolink_core::spkg::parse_file(std::path::Path::new(&p)).unwrap();
    println!("records={} indices={:?}", k.records.len(), k.records.iter().map(|r| r.index).take(6).collect::<Vec<_>>());
}
