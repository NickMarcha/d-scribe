use std::env;
use std::path::Path;

fn main() {
    tauri_build::build();

    // Copy whisper-cli DLLs to target dir so the sidecar can find them at runtime
    let out_dir = env::var("OUT_DIR").unwrap();
    let target_dir = Path::new(&out_dir)
        .ancestors()
        .nth(3)
        .expect("OUT_DIR should be under target/.../build/.../out");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let binaries_dir = Path::new(&manifest_dir).join("binaries");
    let dlls = ["ggml-base.dll", "ggml-cpu.dll", "ggml.dll", "whisper.dll"];
    for dll in &dlls {
        let src = binaries_dir.join(dll);
        if src.exists() {
            let dst = target_dir.join(dll);
            if let Err(e) = std::fs::copy(&src, &dst) {
                eprintln!("cargo:warning=Failed to copy {} to target: {}", dll, e);
            }
        }
    }
}
