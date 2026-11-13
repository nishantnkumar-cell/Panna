//! The Combine files card, as Acrobat's Combine Files: PDFs and pictures in a list that can
//! be put in order, combined into one new PDF with a bookmark for each file, saved, and
//! opened.

use std::path::PathBuf;

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Center, Element, Fill, Task};

use crate::Message;
use crate::icons::{self, Icon};

#[derive(Debug, Clone, Default)]
pub struct Combining {
    pub files: Vec<PathBuf>,
    /// The files are being combined.
    pub working: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Add,
    Added(Vec<PathBuf>),
    Up(usize),
    Down(usize),
    Remove(usize),
    /// Asks where to save, then combines.
    Go,
    SaveTo(Option<PathBuf>),
    Done(Result<PathBuf, String>),
    Close,
}

impl Combining {
    /// Handles `event`; the card closes when this returns `None`.
    pub fn update(mut self, event: Event) -> (Option<Self>, Task<Message>) {
        match event {
            Event::Open => {}
            Event::Add => {
                let task = Task::perform(
                    async {
                        let mut kinds: Vec<&str> = vec!["pdf"];
                        kinds.extend(panna_core::combine::PICTURES);
                        let upper: Vec<String> = kinds.iter().map(|k| k.to_uppercase()).collect();
                        kinds.extend(upper.iter().map(String::as_str));
                        rfd::AsyncFileDialog::new()
                            .set_title("Add files to combine")
                            .add_filter("PDFs and pictures", &kinds)
                            .pick_files()
                            .await
                            .map(|files| files.iter().map(|f| f.path().to_path_buf()).collect())
                            .unwrap_or_default()
                    },
                    |files| Message::Combine(Event::Added(files)),
                );
                return (Some(self), task);
            }
            Event::Added(mut files) => {
                // Chosen together, in name order, as numbered scans and chapters usually are.
                files.sort();
                let room = panna_core::combine::MAX_FILES.saturating_sub(self.files.len());
                self.files.extend(files.into_iter().take(room));
                self.error = None;
            }
            Event::Up(i) if i > 0 && i < self.files.len() => self.files.swap(i, i - 1),
            Event::Down(i) if i + 1 < self.files.len() => self.files.swap(i, i + 1),
            Event::Up(_) | Event::Down(_) => {}
            Event::Remove(i) => {
                if i < self.files.len() && !self.working {
                    self.files.remove(i);
                }
            }
            Event::Go => {
                let name = self
                    .files
                    .first()
                    .and_then(|f| f.file_stem())
                    .map_or_else(|| "Combined".to_owned(), |s| s.to_string_lossy().into_owned());
                let dir = self
                    .files
                    .first()
                    .and_then(|f| f.parent())
                    .map(std::path::Path::to_path_buf);
                let task = Task::perform(
                    async move {
                        let mut dialog = rfd::AsyncFileDialog::new()
                            .set_title("Save the combined PDF as")
                            .add_filter("PDF documents", &["pdf"])
                            .set_file_name(format!("{name} combined.pdf"));
                        if let Some(dir) = dir {
                            dialog = dialog.set_directory(dir);
                        }
                        dialog.save_file().await.map(|f| f.path().to_path_buf())
                    },
                    |to| Message::Combine(Event::SaveTo(to)),
                );
                return (Some(self), task);
            }
            Event::SaveTo(None) => {}
            Event::SaveTo(Some(target)) => {
                if self.files.contains(&target) {
                    self.error = Some("Save it under a new name: that file is one being combined.".into());
                    return (Some(self), Task::none());
                }
                self.working = true;
                self.error = None;
                let files = self.files.clone();
                let (tx, rx) = futures_channel::oneshot::channel();
                std::thread::spawn(move || {
                    let result = panna_core::combine::combine(&files, panna_core::Limits::default(), |_| ())
                        .and_then(|bytes| {
                            std::fs::write(&target, bytes)
                                .map_err(|e| format!("Couldn't write the PDF: {e}"))?;
                            Ok(target)
                        });
                    let _ = tx.send(result);
                });
                let task = Task::perform(rx, |result| {
                    Message::Combine(Event::Done(
                        result.unwrap_or_else(|_| Err("Combining stopped unexpectedly.".into())),
                    ))
                });
                return (Some(self), task);
            }
            Event::Done(Ok(path)) => {
                return (None, Task::done(Message::FileChosen(Some(path))));
            }
            Event::Done(Err(e)) => {
                self.working = false;
                self.error = Some(e);
            }
            Event::Close => {
                if self.working {
                    return (Some(self), Task::none());
                }
                return (None, Task::none());
            }
        }
        (Some(self), Task::none())
    }
}

/// The Combine files card.
pub fn card(state: &Combining) -> Element<'_, Message> {
    let ev = Message::Combine;
    let idle = !state.working;
    let count = state.files.len();
    let list = state
        .files
        .iter()
        .enumerate()
        .fold(column![].spacing(2), |list, (i, file)| {
            let name = file
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let kind = if panna_core::combine::is_picture(file) {
                Icon::AddPicture
            } else {
                Icon::SinglePage
            };
            list.push(
                row![
                    icons::glyph(kind, true),
                    text(name).size(13).width(Fill),
                    icons::button(
                        Icon::MoveUp,
                        "Move up",
                        (idle && i > 0).then_some(ev(Event::Up(i)))
                    ),
                    icons::button(
                        Icon::MoveDown,
                        "Move down",
                        (idle && i + 1 < count).then_some(ev(Event::Down(i)))
                    ),
                    icons::button(
                        Icon::Delete,
                        "Take out of the list",
                        idle.then_some(ev(Event::Remove(i)))
                    ),
                ]
                .spacing(6)
                .align_y(Center),
            )
        });
    let mut body = column![
        text("Combine files").size(16),
        text(
            "Puts PDFs and pictures together into one new PDF, in this order, with a bookmark for \
             each file. Files with a password can't be combined."
        )
        .size(12)
        .style(text::secondary),
    ]
    .spacing(10)
    .width(480);
    body = body.push(if state.files.is_empty() {
        Element::from(text("No files yet.").size(13))
    } else {
        scrollable(list).height(iced::Length::Shrink).into()
    });
    body = body.push(
        container(
            button(
                row![icons::glyph(Icon::PageAdd, true), text("Add files…")]
                    .spacing(6)
                    .align_y(Center),
            )
            .style(button::secondary)
            .on_press_maybe(idle.then_some(ev(Event::Add))),
        )
        .align_left(Fill),
    );
    if state.working {
        body = body.push(text(format!("Combining {count} files…")).size(13));
    }
    if let Some(error) = &state.error {
        body = body.push(text(error.as_str()).size(13).style(text::danger));
    }
    body = body.push(
        row![
            space::horizontal(),
            button("Cancel")
                .style(button::secondary)
                .on_press_maybe(idle.then_some(ev(Event::Close))),
            button("Combine and save as…").on_press_maybe((idle && count >= 2).then_some(ev(Event::Go))),
        ]
        .spacing(8),
    );
    container(body).padding(16).style(container::rounded_box).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_added_in_name_order_and_moved_and_taken_out() {
        let state = Combining::default();
        let (state, _) = state.update(Event::Added(vec!["b.pdf".into(), "a.png".into()]));
        let (state, _) = state.unwrap().update(Event::Added(vec!["c.pdf".into()]));
        let state = state.unwrap();
        assert_eq!(
            state.files,
            [PathBuf::from("a.png"), "b.pdf".into(), "c.pdf".into()]
        );
        let (state, _) = state.update(Event::Down(0));
        let (state, _) = state.unwrap().update(Event::Up(2));
        let (state, _) = state.unwrap().update(Event::Remove(0));
        let state = state.unwrap();
        assert_eq!(state.files, [PathBuf::from("c.pdf"), "a.png".into()]);
        // Moving past the ends does nothing.
        let (state, _) = state.update(Event::Up(0));
        let (state, _) = state.unwrap().update(Event::Down(1));
        assert_eq!(state.unwrap().files, [PathBuf::from("c.pdf"), "a.png".into()]);
    }
}
