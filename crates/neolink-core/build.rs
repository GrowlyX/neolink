fn main() {
    println!("cargo:rerun-if-changed=native/xpc_shim.m");
    println!("cargo:rerun-if-changed=native/h264_shim.m");
    cc::Build::new()
        .file("native/xpc_shim.m")
        .flag("-fobjc-arc")
        .compile("neolink_xpc_shim");
    cc::Build::new()
        .file("native/h264_shim.m")
        .flag("-fobjc-arc")
        .compile("neolink_h264");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=VideoToolbox");
    println!("cargo:rustc-link-lib=framework=CoreMedia");
    println!("cargo:rustc-link-lib=framework=CoreVideo");
}
