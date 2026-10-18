//! Printing through the system's own print dialog.
//!
//! The document, with any unsaved changes, goes to the system as a PDF without its password.
//! On Linux that is the desktop's print portal, which shows the KDE or GNOME print dialog and
//! also works inside Flatpak. Elsewhere the copy opens in the default PDF app to print from.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Numbers print jobs so that no two share a temporary file or portal request.
static JOBS: AtomicUsize = AtomicUsize::new(0);

/// Prints `pdf` named `title`, on a thread of its own because the dialog stays up until the
/// user chooses.
pub async fn print(pdf: Vec<u8>, title: String) -> Result<(), String> {
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(system::print(&pdf, &title));
    });
    rx.await
        .unwrap_or_else(|_| Err("Printing stopped unexpectedly.".into()))
}

/// A temporary file name for print job `job`.
fn temp_path(job: usize) -> PathBuf {
    std::env::temp_dir().join(format!("panna-print-{}-{job}.pdf", std::process::id()))
}

#[cfg(all(unix, not(target_os = "macos")))]
mod system {
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::io::{Seek, SeekFrom, Write};
    use std::os::unix::fs::OpenOptionsExt;

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Value};

    use super::{JOBS, Ordering, temp_path};

    const PORTAL: &str = "org.freedesktop.portal.Desktop";

    pub fn print(pdf: &[u8], title: &str) -> Result<(), String> {
        let job = JOBS.fetch_add(1, Ordering::Relaxed);
        let file =
            readable_copy(pdf, job).map_err(|e| format!("Couldn't prepare the pages to print: {e}"))?;
        match portal(&file, title, job) {
            // Printed, or the dialog was closed without printing.
            Ok(0 | 1) => Ok(()),
            Ok(_) => Err("The pages couldn't be printed.".into()),
            Err(e) => Err(format!("The system print dialog isn't available ({e}).")),
        }
    }

    /// `pdf` in a file only this user can read, already removed from the folder: the portal
    /// reads it through the open file, which lasts as long as anyone holds it.
    fn readable_copy(pdf: &[u8], job: usize) -> std::io::Result<File> {
        let path = temp_path(job);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        let _ = std::fs::remove_file(&path);
        file.write_all(pdf)?;
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    }

    /// Asks the print portal to print `file`. With no settings given, it shows the print
    /// dialog first, and its answer arrives as a Response signal on the request it names:
    /// 0 for printed, 1 for cancelled, 2 for failed.
    fn portal(file: &File, title: &str, job: usize) -> zbus::Result<u32> {
        let conn = Connection::session()?;
        let token = format!("panna_print_{}_{job}", std::process::id());
        let sender = conn
            .unique_name()
            .ok_or_else(|| zbus::Error::Failure("no bus name".into()))?
            .trim_start_matches(':')
            .replace('.', "_");
        // Listen before asking, so the answer can't arrive before anyone is waiting for it.
        let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
        let request = Proxy::new(&conn, PORTAL, path.as_str(), "org.freedesktop.portal.Request")?;
        let mut responses = request.receive_signal("Response")?;

        let print = Proxy::new(
            &conn,
            PORTAL,
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Print",
        )?;
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", token.as_str().into());
        options.insert("modal", true.into());
        let handle: OwnedObjectPath = print.call("Print", &("", title, Fd::from(file), options))?;
        if handle.as_str() != path {
            return Err(zbus::Error::Failure("the portal is too old".into()));
        }
        let response = responses
            .next()
            .ok_or_else(|| zbus::Error::Failure("no answer from the portal".into()))?;
        let (code, _results): (u32, HashMap<String, OwnedValue>) = response.body().deserialize()?;
        Ok(code)
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
mod system {
    use std::process::Command;

    use super::{JOBS, Ordering, temp_path};

    /// Writes `pdf` to a temporary file and opens it in the default PDF app, passing the path
    /// as a single argument, never through a shell.
    pub fn print(pdf: &[u8], _title: &str) -> Result<(), String> {
        let path = temp_path(JOBS.fetch_add(1, Ordering::Relaxed));
        std::fs::write(&path, pdf).map_err(|e| format!("Couldn't prepare the pages to print: {e}"))?;
        let mut command = if cfg!(target_os = "windows") {
            let mut c = Command::new("rundll32");
            c.arg("url.dll,FileProtocolHandler");
            c
        } else {
            Command::new("open")
        };
        command
            .arg(&path)
            .spawn()
            .map(drop)
            .map_err(|e| format!("Couldn't open the pages to print: {e}"))
    }
}
