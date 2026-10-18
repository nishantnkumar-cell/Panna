//! Reading text aloud with the system's own speech: Speech Dispatcher or eSpeak on Linux,
//! `say` on macOS and the built-in speech synthesizer on Windows. The text goes to the
//! program on its standard input, never on a command line.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::widget::{container, row, text};
use iced::{Center, Element, Theme};

use crate::Message;
use crate::icons::{self, Icon};

/// The most text read in one go, in characters.
const MAX_CHARS: usize = 200_000;
const POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub enum Event {
    /// The selected text, or else the current page.
    Page,
    /// From the current page to the end.
    ToEnd,
    Stop,
}

/// Which program speaks, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    SpeechDispatcher,
    Espeak(&'static str),
    Say,
    Windows,
}

impl Engine {
    fn command(self) -> Command {
        match self {
            Engine::SpeechDispatcher => {
                let mut c = Command::new("spd-say");
                // Reads standard input, and waits until it has all been said.
                c.args(["--wait", "--pipe-mode"]);
                c
            }
            Engine::Espeak(name) => {
                let mut c = Command::new(name);
                c.arg("--stdin");
                c
            }
            Engine::Say => Command::new("/usr/bin/say"),
            Engine::Windows => windows_command(),
        }
    }
}

#[cfg(windows)]
fn windows_command() -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // The full path, so no program of the same name elsewhere is run instead.
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let mut c =
        Command::new(std::path::Path::new(&root).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe"));
    c.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "[Console]::InputEncoding = [Text.Encoding]::UTF8; \
         Add-Type -AssemblyName System.Speech; \
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         $s.SetOutputToDefaultAudioDevice(); \
         $s.Speak([Console]::In.ReadToEnd())",
    ])
    .creation_flags(CREATE_NO_WINDOW);
    c
}

#[cfg(not(windows))]
fn windows_command() -> Command {
    Command::new("false")
}

fn engines() -> &'static [Engine] {
    if cfg!(windows) {
        &[Engine::Windows]
    } else if cfg!(target_os = "macos") {
        &[Engine::Say]
    } else {
        &[
            Engine::SpeechDispatcher,
            Engine::Espeak("espeak-ng"),
            Engine::Espeak("espeak"),
        ]
    }
}

/// Speech under way.
#[derive(Debug, Clone)]
struct Speaking {
    engine: Engine,
    child: Arc<Mutex<Child>>,
    stopped: Arc<AtomicBool>,
}

impl Speaking {
    /// Stops at once, so nothing goes on being said once Panna has closed.
    fn halt(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if self.engine == Engine::SpeechDispatcher {
            // What was handed to the speech service is still being said.
            let _ = Command::new("spd-say")
                .arg("--cancel")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// Starts saying `text`; the receiver says whether it was all said (not stopped), or why
/// not.
fn speak(text: &str) -> Result<(Speaking, futures_channel::oneshot::Receiver<Result<bool, String>>), String> {
    let mut started: Option<(Engine, Child)> = None;
    for &engine in engines() {
        match engine
            .command()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                started = Some((engine, child));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("Speech couldn't be started: {e}")),
        }
    }
    let Some((engine, mut child)) = started else {
        return Err(if cfg!(target_os = "linux") {
            "No speech program was found. Reading aloud uses Speech Dispatcher (spd-say) or eSpeak.".into()
        } else {
            "The system's speech couldn't be found.".into()
        });
    };
    let mut said: String = text.chars().take(MAX_CHARS).collect();
    said.push('\n');
    let input = child.stdin.take();
    let speaking = Speaking {
        engine,
        child: Arc::new(Mutex::new(child)),
        stopped: Arc::new(AtomicBool::new(false)),
    };
    let watched = speaking.clone();
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::spawn(move || {
        if let Some(mut input) = input {
            // Written whole before waiting; the pipe closes when `input` is dropped.
            let _ = input.write_all(said.as_bytes());
        }
        let result = loop {
            if watched.stopped.load(Ordering::Relaxed) {
                break Ok(false);
            }
            let status = match watched.child.lock() {
                Ok(mut child) => child.try_wait(),
                Err(_) => break Ok(false),
            };
            match status {
                Ok(Some(_)) if watched.stopped.load(Ordering::Relaxed) => break Ok(false),
                Ok(Some(status)) if status.success() => break Ok(true),
                Ok(Some(_)) => break Err("The system's speech stopped with an error.".into()),
                Ok(None) => std::thread::sleep(POLL),
                Err(e) => break Err(format!("Speech stopped: {e}")),
            }
        };
        let _ = tx.send(result);
    });
    Ok((speaking, rx))
}

#[derive(Debug)]
struct Reading {
    /// The page being read, or `None` for selected text.
    page: Option<usize>,
    to_end: bool,
    /// What's being said.
    stop: Option<Speaking>,
    /// Whether saying has started; not while the page's text is awaited.
    started: bool,
}

/// What's being read aloud.
#[derive(Debug, Default)]
pub struct Reader {
    /// Counts readings, so news from a stopped one is ignored.
    run: u64,
    reading: Option<Reading>,
}

/// What the main view does next.
pub enum Next {
    Nothing,
    /// Wait for the text of this page.
    NeedText(usize),
    /// Show this page, which is being read now.
    Show(usize),
}

impl Reader {
    pub fn active(&self) -> bool {
        self.reading.is_some()
    }

    pub fn stop(&mut self) {
        if let Some(speaking) = self.reading.take().and_then(|r| r.stop) {
            speaking.halt();
        }
        self.run += 1;
    }

    /// Starts reading page `page`, to the end of the document when `to_end`.
    pub fn read_page(&mut self, page: usize, to_end: bool) {
        self.stop();
        self.reading = Some(Reading {
            page: Some(page),
            to_end,
            stop: None,
            started: false,
        });
    }

    /// The page whose text is awaited, if any.
    pub fn waiting_for(&self) -> Option<usize> {
        self.reading.as_ref().filter(|r| !r.started).and_then(|r| r.page)
    }

    fn start(&mut self, text: &str) -> Result<Task, String> {
        let (stop, rx) = speak(text).inspect_err(|_| self.reading = None)?;
        if let Some(r) = &mut self.reading {
            r.stop = Some(stop);
            r.started = true;
        }
        let run = self.run;
        Ok(iced::Task::perform(rx, move |result| Message::SpeechDone {
            run,
            result: result.unwrap_or(Ok(false)),
        }))
    }

    /// Reads `text` (the selection) aloud.
    pub fn read_text(&mut self, text: &str) -> Result<Task, String> {
        self.stop();
        self.reading = Some(Reading {
            page: None,
            to_end: false,
            stop: None,
            started: false,
        });
        self.start(text)
    }

    /// Goes on with the reading, given each page's text where known.
    pub fn go_on(
        &mut self,
        pages: usize,
        text_of: impl Fn(usize) -> Option<String>,
    ) -> Result<(Next, Option<Task>), String> {
        let Some(reading) = &mut self.reading else {
            return Ok((Next::Nothing, None));
        };
        let Some(mut page) = reading.page else {
            return Ok((Next::Nothing, None));
        };
        let first = page;
        loop {
            let Some(text) = text_of(page) else {
                reading.page = Some(page);
                return Ok((Next::NeedText(page), None));
            };
            if !text.trim().is_empty() {
                reading.page = Some(page);
                let task = self.start(&text)?;
                return Ok((Next::Show(page), Some(task)));
            }
            if !reading.to_end || page + 1 >= pages {
                let message = if page == first && !reading.to_end {
                    "There's no text to read on this page."
                } else {
                    "There's no more text to read."
                };
                self.reading = None;
                return Err(message.into());
            }
            page += 1;
        }
    }

    /// A reading finished, all said or not. Returns whether to go on to the next page.
    pub fn done(&mut self, run: u64, all_said: bool, pages: usize) -> bool {
        if run != self.run {
            return false;
        }
        match &mut self.reading {
            Some(Reading {
                page: Some(page),
                to_end: true,
                stop,
                started,
            }) if all_said && *page + 1 < pages => {
                *page += 1;
                *stop = None;
                *started = false;
                true
            }
            _ => {
                self.reading = None;
                false
            }
        }
    }

    pub fn bar(&self) -> Option<Element<'_, Message>> {
        let reading = self.reading.as_ref()?;
        let what = match reading.page {
            Some(page) if reading.to_end => format!("Reading aloud from page {}", page + 1),
            Some(page) => format!("Reading page {} aloud", page + 1),
            None => "Reading the selected text aloud".into(),
        };
        let content = row![
            icons::glyph(Icon::ReadAloud, true),
            text(what).size(13),
            icons::button(
                Icon::StopReading,
                "Stop reading (Ctrl+Shift+E)",
                Some(Message::ReadAloud(Event::Stop))
            ),
        ]
        .spacing(8)
        .align_y(Center);
        Some(
            container(content)
                .padding([4, 8])
                .style(|theme: &Theme| {
                    let palette = theme.extended_palette();
                    container::Style::default()
                        .background(palette.background.base.color)
                        .border(iced::Border {
                            color: palette.background.strong.color,
                            width: 1.0,
                            radius: 8.0.into(),
                        })
                })
                .into(),
        )
    }
}

type Task = iced::Task<Message>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_to_the_end_steps_through_pages() {
        let mut r = Reader::default();
        r.read_page(1, true);
        assert_eq!(r.waiting_for(), Some(1));
        let run = r.run;
        // Pretend page 1 is being said.
        if let Some(reading) = &mut r.reading {
            reading.started = true;
        }
        assert_eq!(r.waiting_for(), None);
        assert!(r.done(run, true, 3));
        assert_eq!(r.waiting_for(), Some(2));
        assert!(!r.done(run, true, 3));
        assert!(!r.active());
    }

    #[test]
    fn stopping_ignores_the_old_reading() {
        let mut r = Reader::default();
        r.read_page(0, true);
        let run = r.run;
        r.stop();
        assert!(!r.active());
        r.read_page(0, true);
        assert!(!r.done(run, true, 3));
        assert!(r.active());
    }

    #[test]
    fn pages_without_text_are_skipped_or_reported() {
        let mut r = Reader::default();
        r.read_page(0, true);
        let texts = [Some(String::new()), None, Some("x".into())];
        let text_of = |i: usize| texts.get(i).cloned().flatten();
        assert!(matches!(r.go_on(3, text_of), Ok((Next::NeedText(1), None))));
        r.read_page(0, false);
        assert!(r.go_on(3, text_of).is_err());
        assert!(!r.active());
    }
}
