// Embed the app icon into tailzip.exe on Windows targets, and patch the UnRAR cross-build.
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=tailzip.rc");
    println!("cargo:rerun-if-changed=assets/tailzip.ico");
    println!("cargo:rerun-if-env-changed=UNRAR_SRC");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let _ = embed_resource::compile("tailzip.rc", embed_resource::NONE);
        // unrar_sys picks its Windows-only source (isnt.cpp) by the *host* OS, so a build from
        // macOS/Linux misses it. Compile it here for the Windows target.
        if !cfg!(windows) {
            cc::Build::new()
                .cpp(true)
                .std("c++14")
                .cpp_link_stdlib(None)
                .warnings(false)
                .define("RARDLL", None)
                .define("RAR_SMP", None)
                // Linked before unrar_sys, so keep every symbol.
                .link_lib_modifier("+whole-archive")
                .file(unrar_src().join("isnt.cpp"))
                .compile("unrar_isnt");
            // It also asks for a dynamic -lpthread, which clashes with the static winpthread
            // that Rust's std links. An empty import library satisfies that request.
            let dir = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("nopthread");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("libpthread.dll.a"), b"!<arch>\n");
            println!("cargo:rustc-link-search=native={}", dir.display());
        }
    }
}

/// vendor/unrar of the unrar_sys crate: $UNRAR_SRC, else the cargo registry copy that
/// matches the version in Cargo.lock.
fn unrar_src() -> PathBuf {
    if let Some(p) = std::env::var_os("UNRAR_SRC") {
        return PathBuf::from(p);
    }
    let lock = std::fs::read_to_string("Cargo.lock").unwrap_or_default();
    let ver = lock
        .split("[[package]]")
        .find(|p| p.contains("name = \"unrar_sys\""))
        .and_then(|p| p.split("version = \"").nth(1))
        .and_then(|v| v.split('"').next())
        .unwrap_or("")
        .to_string();
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
        .unwrap_or_default();
    std::fs::read_dir(home.join("registry/src"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|d| d.path().join(format!("unrar_sys-{ver}/vendor/unrar")))
        .find(|p| p.join("isnt.cpp").is_file())
        .unwrap_or_else(|| {
            panic!("UnRAR sources for unrar_sys {ver} not found; set UNRAR_SRC to its vendor/unrar")
        })
}
