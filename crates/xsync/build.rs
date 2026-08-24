use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=resources/xsync-server.cmd");

    let out_dir = env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR");
    let profile_dir = Path::new(&out_dir)
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("OUT_DIR must be below target/<profile>/build/<package>");
    let destination = profile_dir.join("xsync-server.cmd");

    fs::copy("resources/xsync-server.cmd", &destination)
        .unwrap_or_else(|error| panic!("cannot package {}: {error}", destination.display()));
}
