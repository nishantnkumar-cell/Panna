//! Reading text aloud with the system's own speech: Speech Dispatcher or eSpeak on Linux,
//! `say` on macOS and the built-in speech synthesizer on Windows. The text goes to the
//! program on its standard input, never on a command line.
//!
//! Pausing, as Acrobat's Read Out Loud does, stops the speech program and remembers about
//! where it had got to, from how long it had been speaking; going on starts again from the
//! beginning of that sentence, so a few words may be said twice but none are missed.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::widget::{container, row, text};
use iced::{Center, Element, Theme};

use crate::Message;
use crate::icons::{self, Icon};

/// The most text read in one go, in characters.
const MAX_CHARS: usize = 200_000;
const POLL: Duration = Duration::from_millis(100);
/// Characters said a second, on the slow side of the engines' usual rates (about 175 words
/// a minute), so a reading paused and resumed goes back rather than skips ahead.
const CHARS_PER_SECOND: f32 = 12.0;

#[derive(Debug, Clone)]
pub enum Event {
    /// The selected text, or else the current page.
    Page,
    /// From the current page to the end.
    ToEnd,
    /// Pauses, or goes on after a pause.
    Pause,
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
    /// What this page (or the selection) says, and where in it, in characters, saying last
    /// started, and when.
    text: Vec<char>,
    from: usize,
    since: Option<Instant>,
    /// Paused, at this many characters into `text`.
    paused: Option<usize>,
}

impl Reading {
    fn new(page: Option<usize>, to_end: bool) -> Self {
        Reading {
            page,
            to_end,
            stop: None,
            started: false,
            text: Vec::new(),
            from: 0,
            since: None,
            paused: None,
        }
    }
}

/// Where to start again in `text` after pausing `elapsed` into saying it from `from`: the
/// start of the sentence that was being said.
fn resume_at(text: &[char], from: usize, elapsed: Duration) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let said = (elapsed.as_secs_f32() * CHARS_PER_SECOND) as usize;
    let at = from.saturating_add(said).min(text.len());
    // Back to just after the last sentence end, or line break, before it.
    let mut i = at;
    while i > from {
        let prev = text.get(i - 1).copied().unwrap_or(' ');
        let before = i.checked_sub(2).and_then(|j| text.get(j)).copied().unwrap_or(' ');
        if prev == '\n' || (prev.is_whitespace() && ".!?".contains(before)) {
            break;
        }
        i -= 1;
    }
    i
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

    /// Paused, waiting to go on.
    pub fn paused(&self) -> bool {
        self.reading.as_ref().is_some_and(|r| r.paused.is_some())
    }

    /// Pauses saying, or goes on from about where it was paused.
    pub fn toggle_pause(&mut self) -> Result<Option<Task>, String> {
        let Some(reading) = &mut self.reading else { return Ok(None) };
        if let Some(at) = reading.paused.take() {
            let rest: String = reading.text.get(at..).unwrap_or_default().iter().collect();
            let text = std::mem::take(&mut reading.text);
            return self.start_at(text, at, &rest).map(Some);
        }
        if !reading.started {
            return Ok(None);
        }
        let at = resume_at(
            &reading.text,
            reading.from,
            reading.since.map_or(Duration::ZERO, |t| t.elapsed()),
        );
        reading.paused = Some(at);
        if let Some(speaking) = reading.stop.take() {
            speaking.halt();
        }
        // What the halted speech reports is old news.
        self.run += 1;
        Ok(None)
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
        self.reading = Some(Reading::new(Some(page), to_end));
    }

    /// The page whose text is awaited, if any.
    pub fn waiting_for(&self) -> Option<usize> {
        self.reading.as_ref().filter(|r| !r.started).and_then(|r| r.page)
    }

    fn start(&mut self, text: &str) -> Result<Task, String> {
        let chars: Vec<char> = text.chars().take(MAX_CHARS).collect();
        let said: String = chars.iter().collect();
        self.start_at(chars, 0, &said)
    }

    /// Says `said`, which is `text` from `from` on.
    fn start_at(&mut self, text: Vec<char>, from: usize, said: &str) -> Result<Task, String> {
        let (stop, rx) = speak(said).inspect_err(|_| self.reading = None)?;
        if let Some(r) = &mut self.reading {
            r.stop = Some(stop);
            r.started = true;
            r.text = text;
            r.from = from;
            r.since = Some(Instant::now());
            r.paused = None;
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
        self.reading = Some(Reading::new(None, false));
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
            Some(reading) if all_said && reading.to_end && reading.page.is_some_and(|p| p + 1 < pages) => {
                *reading = Reading::new(reading.page.map(|p| p + 1), true);
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
            _ if reading.paused.is_some() => "Reading aloud paused".to_owned(),
            Some(page) if reading.to_end => format!("Reading aloud from page {}", page + 1),
            Some(page) => format!("Reading page {} aloud", page + 1),
            None => "Reading the selected text aloud".into(),
        };
        let pause = if reading.paused.is_some() {
            icons::button(Icon::Play, "Go on reading", Some(Message::ReadAloud(Event::Pause)))
        } else {
            icons::button(
                Icon::Pause,
                "Pause reading",
                reading.started.then_some(Message::ReadAloud(Event::Pause)),
            )
        };
        let content = row![
            icons::glyph(Icon::ReadAloud, true),
            text(what).size(13),
            pause,
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

    #[test]
    fn a_paused_reading_goes_on_from_the_start_of_its_sentence() {
        let text: Vec<char> = "One two. Three four five six seven.\nEight nine.".chars().collect();
        // Twelve characters a second: a second in is inside the second sentence.
        assert_eq!(resume_at(&text, 0, Duration::from_secs(1)), 9);
        assert_eq!(resume_at(&text, 0, Duration::ZERO), 0);
        // Past the line break, back to it; never before where saying started.
        assert_eq!(resume_at(&text, 0, Duration::from_secs(4)), 36);
        assert_eq!(resume_at(&text, 20, Duration::from_millis(100)), 20);
        assert_eq!(resume_at(&text, 0, Duration::from_secs(100)), 36);
    }

    #[test]
    fn pausing_ignores_what_the_halted_speech_reports() {
        let mut r = Reader::default();
        r.read_page(0, true);
        if let Some(reading) = &mut r.reading {
            reading.started = true;
            reading.text = "Hello there.".chars().collect();
        }
        let run = r.run;
        assert!(matches!(r.toggle_pause(), Ok(None)));
        assert!(r.paused());
        assert!(!r.done(run, false, 3));
        assert!(r.active() && r.paused());
    }
}
