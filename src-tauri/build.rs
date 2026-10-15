fn main() {
    // tauri-build embeds its app manifest into BIN targets only
    // (`cargo:rustc-link-arg-bins`). Test harnesses would get no manifest at
    // all, and any harness linking the tauri runtime fails to load with
    // STATUS_ENTRYPOINT_NOT_FOUND: the MCP code monomorphizes
    // tauri-runtime-wry, whose dialog module imports `TaskDialogIndirect`
    // from Common-Controls **v6** — without a manifest, comctl32 resolves to
    // v5. So: tell tauri-build to skip its bin-only manifest and provide the
    // identical one ourselves for every target (a duplicate embed in the bin
    // would fail with CVT1100 instead).
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    if let Err(error) = tauri_build::try_build(attributes) {
        panic!("failed to run tauri-build: {error}");
    }

    #[cfg(windows)]
    {
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
