fn main() {
    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let icon = manifest_dir.join("../../assets/tokenbar.ico");
    println!("cargo:rerun-if-changed={}", icon.display());

    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }
    embed_windows_icon(&icon);
}

/// Explorer reads icon resource 1 from the exe. `winres` compiles that on Windows.
#[cfg(windows)]
fn embed_windows_icon(icon: &std::path::Path) {
    let path = icon.to_str().expect("tokenbar.ico path is utf-8");
    let mut res = winres::WindowsResource::new();
    res.set_icon(path);
    if let Err(err) = res.compile() {
        panic!("failed to embed {}: {err}", icon.display());
    }
}

#[cfg(not(windows))]
fn embed_windows_icon(_icon: &std::path::Path) {}
