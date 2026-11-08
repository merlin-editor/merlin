//! `cfg(desktop_linux)`: Linux and the BSDs, whose desktops are Wayland or
//! X11, with a selection clipboard; not macOS, iOS or Android, which are
//! Unix too.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(desktop_linux)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let unix = std::env::var("CARGO_CFG_UNIX").is_ok();
    if unix && !matches!(os.as_str(), "macos" | "ios" | "android") {
        println!("cargo::rustc-cfg=desktop_linux");
    }
}
