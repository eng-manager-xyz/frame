use std::{env, path::PathBuf, process::Command};

fn main() {
    if std::env::var_os("CARGO_FEATURE_TAURI_APP").is_some() {
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&["bootstrap_main", "finalize_instant"]),
        ))
        .expect("Frame desktop ACL generation failed");
    }

    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || env::var_os("CARGO_FEATURE_MACOS_NATIVE").is_none()
    {
        return;
    }

    // screencapturekit links a static Swift bridge. The final application must
    // carry the Swift runtime search paths because dependency build-script
    // linker arguments do not propagate to this executable boundary.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    if let Some(runtime) = xcode_swift_runtime() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", runtime.display());
    }
}

fn xcode_swift_runtime() -> Option<PathBuf> {
    let output = Command::new("xcrun")
        .args(["--toolchain", "default", "--find", "swift"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let swift = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    let toolchain_usr = swift.parent()?.parent()?;
    let runtime = toolchain_usr.join("lib/swift/macosx");
    runtime.is_dir().then_some(runtime)
}
