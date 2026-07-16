fn main() {
    if std::env::var_os("CARGO_FEATURE_TAURI_APP").is_some() {
        tauri_build::try_build(
            tauri_build::Attributes::new()
                .app_manifest(tauri_build::AppManifest::new().commands(&["bootstrap_main"])),
        )
        .expect("Frame desktop ACL generation failed");
    }
}
