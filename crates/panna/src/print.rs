//! Printing through the system's own print dialog.
//!
//! The document, with any unsaved changes, goes to the system as a PDF without its password.
//! On Linux that is the desktop's print portal, which shows the KDE or GNOME print dialog and
//! also works inside Flatpak. Windows and macOS show their own print dialogs through the
//! `panna-print` crate.

use iced::Task;

use crate::Message;

/// Prints `pdf` named `title`, with page `current` as the one on screen.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn start(pdf: Vec<u8>, title: String, _current: usize) -> Task<Message> {
    Task::perform(on_thread(move || system::print(&pdf, &title)), Message::Printed)
}

/// Prints `pdf` named `title`, with page `current` as the one on screen. The dialog belongs to
/// the main window, and the pages are drawn on a thread of their own.
#[cfg(windows)]
pub fn start(pdf: Vec<u8>, title: String, current: usize) -> Task<Message> {
    use iced::window;
    use raw_window_handle::RawWindowHandle;

    window::oldest()
        .and_then(|id| {
            window::run(id, |w| match w.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
                _ => None,
            })
        })
        .then(move |owner| {
            let (pdf, title) = (pdf.clone(), title.clone());
            Task::perform(
                on_thread(move || native(&pdf, &title, current, owner)),
                Message::Printed,
            )
        })
}

/// Prints `pdf` named `title`, with page `current` as the one on screen. The print panel is
/// AppKit's, which has to run on the main thread, so this waits for it.
#[cfg(target_os = "macos")]
pub fn start(pdf: Vec<u8>, title: String, current: usize) -> Task<Message> {
    Task::done(Message::Printed(native(&pdf, &title, current, None)))
}

/// Runs `work` on a thread of its own, because a print dialog stays up until the user chooses.
#[cfg(not(target_os = "macos"))]
async fn on_thread(work: impl FnOnce() -> Result<(), String> + Send + 'static) -> Result<(), String> {
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.await
        .unwrap_or_else(|_| Err("Printing stopped unexpectedly.".into()))
}

/// The system print dialog from `panna-print`, drawing pages from the print copy itself.
#[cfg(any(windows, target_os = "macos"))]
fn native(pdf: &[u8], title: &str, current: usize, owner: Option<isize>) -> Result<(), String> {
    use panna_core::{Isolated, Limits};
    use panna_print::{Bitmap, Job, Page};

    // Drawn in the locked-down process, like the pages on screen.
    let (mut doc, info) = Isolated::open(pdf.to_vec(), None, Limits::default())
        .map_err(|e| format!("The pages couldn't be prepared for printing: {e}"))?;
    let pages: Vec<Page> = info
        .pages
        .iter()
        .map(|p| Page {
            width: p.width,
            height: p.height,
        })
        .collect();
    let mut render = |index: usize, scale: f32| {
        doc.render(index, scale).ok().map(|page| Bitmap {
            width: page.width,
            height: page.height,
            rgba: page.rgba,
        })
    };
    panna_print::print(Job {
        title,
        pdf,
        pages: &pages,
        current,
        owner,
        render: &mut render,
    })
    .map(drop)
}

#[cfg(all(unix, not(target_os = "macos")))]
mod system {
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::io::{Seek, SeekFrom, Write};
    use std::os::unix::fs::OpenOptionsExt;

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Value};

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Numbers print jobs so that no two share a temporary file or portal request.
    static JOBS: AtomicUsize = AtomicUsize::new(0);

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
        let path: PathBuf =
            std::env::temp_dir().join(format!("panna-print-{}-{job}.pdf", std::process::id()));
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
