// Embed the app icon into tailzip.exe on Windows targets.
fn main() {
    println!("cargo:rerun-if-changed=tailzip.rc");
    println!("cargo:rerun-if-changed=assets/tailzip.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let _ = embed_resource::compile("tailzip.rc", embed_resource::NONE);
    }
}
