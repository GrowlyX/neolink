//! Encode→decode every awkward damage-rect size and print the coded size.
//!
//! `cargo run -p neolink-core --example h264_sizes`

fn main() {
    for (w, h) in [
        (1u16, 1u16),
        (1, 2),
        (2, 1),
        (17, 16),
        (16, 17),
        (17, 17),
        (15, 15),
        (9, 31),
        (33, 33),
        (1919, 1079),
        (1920, 1080),
    ] {
        let mut e = match neolink_core::h264::Encoder::new(w, h) {
            Ok(e) => e,
            Err(e) => {
                println!("{w}x{h}: new err {e}");
                continue;
            }
        };
        let f = match e.encode(&neolink_core::h264::solid(w, h, [9, 9, 9, 255])) {
            Ok(f) => f,
            Err(e) => {
                println!("{w}x{h}: enc err {e}");
                continue;
            }
        };
        let mut d = neolink_core::h264::Decoder::new(w, h).unwrap();
        let ok = d.decode(&f.bytes);
        println!(
            "{w}x{h}: decode={ok} got={:?} coded={:?}",
            d.decoded_size(),
            e.coded_size()
        );
    }
}
