//! Files opened from Finder on macOS (see `panna_print::finder`), waiting in a channel until
//! the app is ready to open them.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use futures_channel::mpsc;
use iced::Subscription;
use iced::futures::{StreamExt, stream};

use crate::Message;

type Opened = (
    mpsc::UnboundedSender<PathBuf>,
    Mutex<Option<mpsc::UnboundedReceiver<PathBuf>>>,
);

fn channel() -> &'static Opened {
    static OPENED: OnceLock<Opened> = OnceLock::new();
    OPENED.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded();
        (tx, Mutex::new(Some(rx)))
    })
}

/// Gets ready for files from Finder. Call it on the main thread before the app starts.
pub fn install() {
    panna_print::finder::on_open(|path| {
        let _ = channel().0.unbounded_send(path);
    });
}

/// Each file Finder asks to open, as it arrives.
pub fn subscription() -> Subscription<Message> {
    Subscription::run(|| {
        let files = channel().1.lock().ok().and_then(|mut rx| rx.take());
        match files {
            Some(rx) => rx.map(|path| Message::FileChosen(Some(path))).boxed(),
            None => stream::empty().boxed(),
        }
    })
}
