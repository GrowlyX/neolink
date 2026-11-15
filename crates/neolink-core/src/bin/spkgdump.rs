//! Dump an `.spkg`: metadata, fragments, frame table and where it stops.
fn main() {
    for p in std::env::args().skip(1) {
        let name = std::path::Path::new(&p)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        match neolink_core::spkg::parse_file(std::path::Path::new(&p)) {
            Ok(k) => {
                println!(
                    "{name}: name={:?} rev={:?} fields={} records={} stream={}",
                    k.name(),
                    k.text(neolink_core::spkg::Field::Revision),
                    k.fields.len(),
                    k.records.len(),
                    k.stream().len()
                );
                for r in &k.records {
                    println!("   fragment idx={} bytes={}", r.index, r.body.len());
                }
                match k.frames() {
                    Ok(f) => {
                        let data: usize = f.frames.iter().map(Vec::len).sum();
                        println!(
                            "   frames={} data={} trailing={}",
                            f.frames.len(),
                            data,
                            f.rest
                        );
                        if let Some(last) = f.frames.last() {
                            println!("   last {:02x?}", &last[..16.min(last.len())]);
                        }
                    }
                    Err(e) => println!("   frames: {e}"),
                }
            }
            Err(e) => println!("{name}: ERR {e}"),
        }
    }
}
