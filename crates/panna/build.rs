//! Embeds the app icon in the Windows executable so Explorer, the taskbar and shortcuts show it.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../packaging/windows/panna.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/windows/panna.ico")
            .set("ProductName", "Panna")
            .set("FileDescription", "Panna PDF viewer");
        // A missing resource compiler only loses the icon, so warn rather than fail the build.
        if let Err(e) = res.compile() {
            println!("cargo:warning=Couldn't embed the app icon: {e}");
        }
    }
}
