//! Dump an `.spkg`: fields, record count and payload size, then try each decoder.
fn main() {
    for p in std::env::args().skip(1) {
        let name = std::path::Path::new(&p).file_name().unwrap().to_string_lossy().to_string();
        match neolink_core::spkg::parse_file(std::path::Path::new(&p)) {
            Ok(k) => {
                println!("{name}: name={:?} rev={:?} fields={} records={} payload={}",
                    k.name(), k.text(neolink_core::spkg::Field::Revision),
                    k.fields.len(), k.records.len(), k.payload().len());
                for (i, c) in k.payload_candidates().iter().enumerate() {
                    match neolink_core::spkg::inflate(c) {
                        Ok(out) => println!("   skip {i}: {} bytes", out.len()),
                        Err(_) => {}
                    }
                }
            }
            Err(e) => println!("{name}: ERR {e}"),
        }
    }
}
