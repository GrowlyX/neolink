use nusb::MaybeFuture;
fn main() {
    let devs = nusb::list_devices().wait().unwrap();
    let mut n = 0;
    for dev in devs {
        n += 1;
        println!(
            "{:04x}:{:04x} {} | bus {} addr {} class {:02x}",
            dev.vendor_id(),
            dev.product_id(),
            dev.product_string().unwrap_or("?"),
            dev.bus_id(),
            dev.device_address(),
            dev.class()
        );
        for i in dev.interfaces() {
            println!(
                "   iface {} class {:02x} sub {:02x} proto {:02x}",
                i.interface_number(),
                i.class(),
                i.subclass(),
                i.protocol()
            );
        }
    }
    println!("total {n}");
}
