use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    if cfg!(target_os = "windows") {
        if let Err(e) = setup_libmpv_windows() {
            // Emit as a hard error if we're actually compiling Rust code
            // that depends on libmpv (i.e. not just running `cargo metadata`).
            // Cargo always runs build.rs, so a panic here aborts the build.
            panic!("\n\nPlayerSage build error: {e}\n\n\
                    To fix: from the project root, run\n  \
                    pwsh -File ./scripts/setup-libmpv.ps1\n\n");
        }
    }
    tauri_build::build()
}

fn setup_libmpv_windows() -> Result<(), String> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?);
    let vendor_dir = manifest_dir.join("vendor").join("libmpv");
    let resources_dir = manifest_dir.join("resources");

    let dll = vendor_dir.join("libmpv-2.dll");
    let lib = vendor_dir.join("mpv.lib");
    let header = vendor_dir.join("include").join("mpv").join("client.h");

    if !dll.exists() || !lib.exists() || !header.exists() {
        return Err(format!(
            "libmpv not found in {}. Missing one or more of: \
             libmpv-2.dll, mpv.lib, include/mpv/client.h. \
             Run scripts\\setup-libmpv.ps1 first.",
            vendor_dir.display()
        ));
    }

    fs::create_dir_all(&resources_dir).map_err(|e| e.to_string())?;
    copy_if_different(&dll, &resources_dir.join("libmpv-2.dll"))?;
    if let Some(target_dir) = target_artifact_dir() {
        copy_if_different(&dll, &target_dir.join("libmpv-2.dll"))?;
    }

    println!("cargo:rustc-link-search=native={}", vendor_dir.display());
    println!("cargo:rustc-link-lib=dylib=mpv");
    println!("cargo:rustc-env=MPV_SOURCE={}", vendor_dir.display());
    println!("cargo:rerun-if-changed=vendor/libmpv/libmpv-2.dll");
    println!("cargo:rerun-if-changed=vendor/libmpv/mpv.lib");
    println!("cargo:rerun-if-changed=build.rs");
    Ok(())
}

fn target_artifact_dir() -> Option<PathBuf> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").ok()?);
    let mut p = out_dir.as_path();
    for _ in 0..3 {
        p = p.parent()?;
    }
    Some(p.to_path_buf())
}

fn copy_if_different(src: &Path, dst: &Path) -> Result<(), String> {
    let need = match (fs::metadata(src), fs::metadata(dst)) {
        (Ok(s), Ok(d)) => s.len() != d.len(),
        _ => true,
    };
    if need {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::copy(src, dst).map_err(|e| format!("copy {src:?} -> {dst:?}: {e}"))?;
    }
    Ok(())
}
