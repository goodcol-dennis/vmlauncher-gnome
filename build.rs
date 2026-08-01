fn main() {
    match pkg_config::Config::new().probe("spice-client-glib-2.0") {
        Ok(_) => {}
        Err(e) => {
            eprintln!("cargo:warning=pkg-config failed for spice-client-glib-2.0: {e}");
            eprintln!(
                "cargo:warning=Linking directly to soname — install libspice-client-glib-2.0-dev for proper build"
            );
            println!("cargo:rustc-link-search=/usr/lib/x86_64-linux-gnu");
            println!("cargo:rustc-link-lib=dylib:+verbatim=libspice-client-glib-2.0.so.8");
        }
    }
}
