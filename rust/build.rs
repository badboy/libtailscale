use std::process::Command;

fn main() {
    let status = Command::new("go")
        .args(["build", "-buildmode=c-archive", ".."])
        .status()
        .expect("can't build go library");

    assert!(status.success(), "failed to build go library");
}
