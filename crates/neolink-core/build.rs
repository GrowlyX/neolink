fn main() {
    println!("cargo:rerun-if-changed=native/xpc_shim.m");
    cc::Build::new()
        .file("native/xpc_shim.m")
        .flag("-fobjc-arc")
        .compile("neolink_xpc_shim");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=compression");
}
