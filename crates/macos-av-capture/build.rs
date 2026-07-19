use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    // The published ScreenCaptureKit crate links a static Swift bridge. Cargo
    // does not propagate dependency link arguments to this crate's test and
    // example executables, so repeat both Swift runtime search paths here.
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
