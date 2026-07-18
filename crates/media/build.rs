use std::{env, path::Path, process::Command};

fn main() {
    for variable in [
        "PKG_CONFIG",
        "PKG_CONFIG_PATH",
        "PKG_CONFIG_LIBDIR",
        "PKG_CONFIG_SYSROOT_DIR",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    println!("cargo:rerun-if-env-changed=DOCS_RS");
    println!("cargo:rerun-if-env-changed=FRAME_GSTREAMER_COMPILE_ONLY");
    if env::var_os("DOCS_RS").is_some() || env::var_os("FRAME_GSTREAMER_COMPILE_ONLY").is_some() {
        // The gtk-rs sys crates use DOCS_RS as their documented compile-only
        // escape hatch. FRAME_GSTREAMER_COMPILE_ONLY scopes the same unavailable
        // plugin-directory contract to Frame's own build script so shell-only
        // desktop lanes do not disable unrelated native (for example Objective-C)
        // link directives. Runtime media builds deliberately leave both unset
        // and still require an audited, existing plugin directory from pkg-config.
        let unavailable_directory = if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            r"C:\frame-gstreamer-unavailable"
        } else {
            "/frame-gstreamer-unavailable"
        };
        println!("cargo:rustc-env=FRAME_BUILD_GSTREAMER_PLUGIN_DIR={unavailable_directory}");
        return;
    }

    let pkg_config = env::var_os("PKG_CONFIG").unwrap_or_else(|| "pkg-config".into());
    let output = Command::new(pkg_config)
        .args(["--variable=pluginsdir", "gstreamer-1.0"])
        .output()
        .expect("pkg-config is required to locate the audited GStreamer plugin directory");
    assert!(
        output.status.success(),
        "pkg-config could not locate the GStreamer plugin directory"
    );
    let plugin_directory = String::from_utf8(output.stdout)
        .expect("pkg-config returned a non-UTF-8 GStreamer plugin directory");
    let plugin_directory = plugin_directory.trim_end_matches(['\r', '\n']);
    assert!(
        !plugin_directory.is_empty()
            && !plugin_directory.contains(['\r', '\n'])
            && Path::new(plugin_directory).is_absolute(),
        "pkg-config returned an invalid GStreamer plugin directory"
    );
    let plugin_directory = std::fs::canonicalize(plugin_directory)
        .expect("the GStreamer plugin directory reported by pkg-config must exist");
    let plugin_directory = plugin_directory
        .to_str()
        .expect("the GStreamer plugin directory must be UTF-8");
    println!("cargo:rustc-env=FRAME_BUILD_GSTREAMER_PLUGIN_DIR={plugin_directory}");
}
