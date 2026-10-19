//! Following links: page jumps, and handing web and mail addresses to the system's browser or
//! mail program after the user agrees. Panna itself never connects to anything.

use std::io;
use std::process::Command;

use panna_core::{Link, safe_uri};

/// The link under a point on `page`, in page points.
pub fn at(links: &[Link], page: usize, (x, y): (f32, f32)) -> Option<usize> {
    links.iter().position(|l| l.page == page && l.at.contains(x, y))
}

/// Opens a web or mail address with the system's default handler. The address is passed as a
/// single argument, never through a shell.
pub fn open(uri: &str) -> io::Result<()> {
    if !safe_uri(uri) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a web or mail address",
        ));
    }
    let mut command = if cfg!(target_os = "windows") {
        // The full path, so a rundll32.exe placed elsewhere (next to Panna, say) isn't run.
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let mut c = Command::new(std::path::Path::new(&root).join("System32\\rundll32.exe"));
        c.arg("url.dll,FileProtocolHandler");
        c
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        // Also works inside Flatpak, where xdg-open goes through the desktop portal.
        Command::new("xdg-open")
    };
    command.arg(uri).spawn().map(drop)
}
