//! Scanning pages into a new PDF, with the system's own scanning: SANE's `scanimage` on Linux,
//! and the Windows scan dialog (Windows Image Acquisition, through PowerShell) on Windows.
//! Each scan is a page; when they're all in, they become a PDF like pictures do.

use std::io::Read as _;
use std::process::{Command, Stdio};

use iced::widget::{button, checkbox, column, container, pick_list, row, space, text};
use iced::{Element, Theme};

use crate::Message;

/// Pages scanned at most into one PDF.
pub const MAX_PAGES: usize = 500;
/// A scanned page is never larger than this.
const MAX_PAGE_BYTES: u64 = 200 * 1024 * 1024;

/// A scanner, by the name the system knows it by and the name shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub label: String,
}

impl std::fmt::Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Dots per inch, as the list names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dpi(pub u32);

impl Dpi {
    const ALL: [Dpi; 3] = [Dpi(150), Dpi(300), Dpi(600)];
}

impl std::fmt::Display for Dpi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} dpi", self.0)
    }
}

#[derive(Debug, Clone)]
pub struct Scanning {
    /// The scanners found, once looked for (Linux only; Windows asks in its own dialog).
    pub devices: Option<Result<Vec<Device>, String>>,
    pub device: Option<Device>,
    pub dpi: Dpi,
    pub colour: bool,
    /// The pages scanned so far, as PNG files.
    pub pages: Vec<Vec<u8>>,
    pub busy: bool,
    pub error: Option<String>,
}

impl Default for Scanning {
    fn default() -> Self {
        Self {
            devices: None,
            device: None,
            dpi: Dpi(300),
            colour: true,
            pages: Vec::new(),
            busy: false,
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Found(Result<Vec<Device>, String>),
    Device(Device),
    Dpi(Dpi),
    Colour(bool),
    Scan,
    /// A page scanned (`None` when the scan was cancelled), or why it couldn't be.
    Scanned(Result<Option<Vec<u8>>, String>),
    RemoveLast,
    Done,
    Cancel,
}

/// Whether scanning works in this build at all, and if not, why.
pub fn unavailable() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some(
            "Panna can't scan on macOS yet. Scan to pictures with Image Capture, then use Make a \
             PDF from pictures.",
        )
    } else if !cfg!(windows) && std::env::var_os("FLATPAK_ID").is_some() {
        Some(
            "Scanners can't be reached from the Flatpak version of Panna yet. Scan to pictures \
             with your scanning app, then use Make a PDF from pictures.",
        )
    } else {
        None
    }
}

/// Runs `command`, and gives what it wrote, up to `limit` bytes, or what went wrong.
fn output(mut command: Command, limit: u64) -> Result<Vec<u8>, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        stdout
            .take(limit + 1)
            .read_to_end(&mut out)
            .map_err(|e| e.to_string())?;
    }
    if out.len() as u64 > limit {
        let _ = child.kill();
        let _ = child.wait();
        return Err("The scan was too large.".into());
    }
    let mut err = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.by_ref().take(64 * 1024).read_to_string(&mut err);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(out)
    } else {
        let last = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        Err(if last.is_empty() {
            format!("The scanner program stopped ({status}).")
        } else {
            last.to_owned()
        })
    }
}

/// The scanners SANE can reach.
pub fn devices() -> Result<Vec<Device>, String> {
    let mut command = Command::new("scanimage");
    // A tab between the device's name and its vendor and model, one scanner a line.
    command.arg("--formatted-device-list=%d\t%v %m%n");
    let out = output(command, 1024 * 1024).map_err(|e| {
        if e.contains("No such file") || e.contains("not found") {
            "Scanning needs SANE's scanimage program, in the sane-utils (or sane-backends) \
             package."
                .to_owned()
        } else {
            e
        }
    })?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| {
            let (id, label) = line.split_once('\t')?;
            let label = label.trim();
            Some(Device {
                id: id.trim().to_owned(),
                label: if label.is_empty() { id.trim() } else { label }.to_owned(),
            })
        })
        .filter(|d| !d.id.is_empty())
        .take(50)
        .collect())
}

/// Scans a page with SANE, as a PNG file.
fn scan_sane(device: &str, dpi: u32, colour: bool) -> Result<Option<Vec<u8>>, String> {
    let mut command = Command::new("scanimage");
    // Each setting is one argument, value included, so none can be taken for another.
    command.args([
        format!("--device-name={device}"),
        "--format=png".to_owned(),
        format!("--resolution={dpi}"),
        format!("--mode={}", if colour { "Color" } else { "Gray" }),
    ]);
    output(command, MAX_PAGE_BYTES).map(Some)
}

/// Scans a page through the Windows scan dialog, as a PNG file; `None` when it's cancelled.
#[cfg(windows)]
fn scan_windows() -> Result<Option<Vec<u8>>, String> {
    use std::io::Write as _;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let file = std::env::temp_dir().join(format!(
        "panna-scan-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let mut command =
        Command::new(std::path::Path::new(&root).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe"));
    // Scans with the scanner's own dialog as a bitmap (which every scanner can give), turns it
    // into PNG and saves it where standard input says. Exit code 2 means cancelled.
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ErrorActionPreference = 'Stop'; \
             [Console]::InputEncoding = [Text.Encoding]::UTF8; \
             $out = [Console]::In.ReadLine(); \
             $dialog = New-Object -ComObject WIA.CommonDialog; \
             $image = $dialog.ShowAcquireImage(1, 0, 0, '{B96B3CAB-0728-11D3-9D7B-0000F81EF32E}', $false, $true, $false); \
             if ($image -eq $null) { exit 2 }; \
             $process = New-Object -ComObject WIA.ImageProcess; \
             $process.Filters.Add($process.FilterInfos.Item('Convert').FilterID); \
             $process.Filters.Item(1).Properties.Item('FormatID').Value = '{B96B3CAF-0728-11D3-9D7B-0000F81EF32E}'; \
             $image = $process.Apply($image); \
             $image.SaveFile($out)",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{}", file.display());
    }
    let mut err = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.by_ref().take(64 * 1024).read_to_string(&mut err);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.code() == Some(2) {
        return Ok(None);
    }
    let result = if status.success() {
        match std::fs::metadata(&file) {
            Ok(m) if m.len() > MAX_PAGE_BYTES => Err("The scan was too large.".to_owned()),
            Ok(_) => std::fs::read(&file).map(Some).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        }
    } else if err.contains("0x80210015") || err.contains("No WIA device") {
        Err("No scanner was found. Check that it's on and connected.".to_owned())
    } else {
        let first = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        Err(if first.is_empty() {
            "The scan didn't work.".to_owned()
        } else {
            first.to_owned()
        })
    };
    let _ = std::fs::remove_file(&file);
    result
}

#[cfg(not(windows))]
fn scan_windows() -> Result<Option<Vec<u8>>, String> {
    Err("Not on Windows.".into())
}

/// Scans a page, as the card says, off the interface's thread.
pub async fn scan(device: Option<String>, dpi: u32, colour: bool) -> Result<Option<Vec<u8>>, String> {
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::spawn(move || {
        let result = if cfg!(windows) {
            scan_windows()
        } else {
            match device {
                Some(device) => scan_sane(&device, dpi, colour),
                None => Err("Choose a scanner first.".to_owned()),
            }
        };
        let _ = tx.send(result);
    });
    rx.await.unwrap_or_else(|_| Err("The scan stopped.".into()))
}

/// Looks for scanners, off the interface's thread.
pub async fn find() -> Result<Vec<Device>, String> {
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(devices());
    });
    rx.await
        .unwrap_or_else(|_| Err("Looking for scanners stopped.".into()))
}

pub fn card(state: &Scanning) -> Element<'_, Message> {
    let ev = Message::Scan;
    let small = |s: String| text(s).size(13);
    let mut body = column![
        text("Scan pages").size(16),
        small(
            if cfg!(windows) {
                "Each scan becomes a page. Windows asks which scanner and settings to use."
            } else {
                "Each scan becomes a page. When they're all in, make them into a PDF."
            }
            .to_owned()
        ),
    ]
    .spacing(10)
    .width(440);
    if !cfg!(windows) {
        match &state.devices {
            None => body = body.push(small("Looking for scanners…".into()).style(text::secondary)),
            Some(Err(e)) => body = body.push(small(e.clone())),
            Some(Ok(list)) if list.is_empty() => {
                body = body.push(small(
                    "No scanners were found. Check that the scanner is on and connected, and that \
                     SANE supports it."
                        .into(),
                ));
            }
            Some(Ok(list)) => {
                body = body.push(
                    row![
                        text("Scanner").size(13),
                        pick_list(list.clone(), state.device.clone(), move |d| ev(Event::Device(d)))
                            .text_size(13)
                            .width(iced::Fill),
                    ]
                    .spacing(8)
                    .align_y(iced::Center),
                );
                body = body.push(
                    row![
                        pick_list(Dpi::ALL, Some(state.dpi), move |v| ev(Event::Dpi(v))).text_size(13),
                        checkbox(state.colour)
                            .label("In colour")
                            .on_toggle(move |v| ev(Event::Colour(v))),
                    ]
                    .spacing(12)
                    .align_y(iced::Center),
                );
            }
        }
    }
    let count = state.pages.len();
    let status = if state.busy {
        "Scanning…".to_owned()
    } else {
        match count {
            0 => "No pages scanned yet.".to_owned(),
            1 => "1 page scanned.".to_owned(),
            n => format!("{n} pages scanned."),
        }
    };
    let can_scan = !state.busy && count < MAX_PAGES && (cfg!(windows) || state.device.is_some());
    body = body.push(
        row![
            small(status).width(iced::Fill),
            button("Remove the last")
                .style(button::secondary)
                .on_press_maybe((count > 0 && !state.busy).then_some(ev(Event::RemoveLast))),
            button(if count == 0 { "Scan a page" } else { "Scan another" })
                .on_press_maybe(can_scan.then_some(ev(Event::Scan))),
        ]
        .spacing(8)
        .align_y(iced::Center),
    );
    if let Some(error) = &state.error {
        body = body.push(small(error.clone()).style(|theme: &Theme| text::Style {
            color: Some(theme.extended_palette().danger.base.color),
        }));
    }
    body = body.push(
        row![
            space::horizontal(),
            button("Cancel")
                .style(button::secondary)
                .on_press(ev(Event::Cancel)),
            button("Make a PDF…").on_press_maybe((count > 0 && !state.busy).then_some(ev(Event::Done))),
        ]
        .spacing(8),
    );
    container(body)
        .padding(16)
        .style(|theme: &Theme| container::rounded_box(theme))
        .into()
}
