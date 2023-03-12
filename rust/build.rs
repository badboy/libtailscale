use std::env;
use std::process::Command;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let outdir = env::var("OUT_DIR").unwrap();
    let out = format!("{outdir}/libtailscale.a");
    let status = Command::new("go")
        .args(["build", "-buildmode=c-archive", "-o", &out, &manifest_dir])
        .status()
        .expect("can't build go library");

    assert!(status.success(), "failed to build go library");

    println!("cargo:rustc-link-search={outdir}");
    println!("cargo:rustc-link-lib=static=tailscale");

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-flags=-l framework=CoreFoundation -l framework=Security");
    }
}
