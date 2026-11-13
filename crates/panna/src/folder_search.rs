//! The Search a folder card, as Acrobat's Advanced Search: finds words in every PDF in a
//! folder (and the folders inside it, if wanted), one file at a time so it can show how far
//! it has got and stop, and lists each match with the text around it. Picking one opens the
//! file at that page.

use std::path::PathBuf;

use iced::widget::{
    Column, button, checkbox, column, container, rich_text, row, scrollable, space, span, text, text_input,
};
use iced::{Center, Color, Element, Fill, Task};
use panna_core::FindOptions;
use panna_core::folder::Found;

use crate::Message;
use crate::icons::{self, Icon};

/// Matches listed at most, across all files.
const MAX_LISTED: usize = 1_000;
const MATCH: Color = Color::from_rgb8(0xFF, 0xD8, 0x4D);

#[derive(Debug, Clone)]
pub struct FolderSearch {
    pub query: String,
    pub options: FindOptions,
    pub subfolders: bool,
    pub folder: Option<PathBuf>,
    run: Option<Run>,
}

impl Default for FolderSearch {
    fn default() -> Self {
        Self {
            query: String::new(),
            options: FindOptions::default(),
            subfolders: true,
            folder: None,
            run: None,
        }
    }
}

#[derive(Debug, Clone)]
struct Run {
    query: String,
    files: Vec<PathBuf>,
    done: usize,
    /// Each file with matches, and its matches.
    found: Vec<(PathBuf, Vec<Found>)>,
    failed: Vec<(String, String)>,
    stopped: bool,
}

impl Run {
    fn finished(&self) -> bool {
        self.stopped || self.done >= self.files.len()
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Query(String),
    MatchCase(bool),
    WholeWord(bool),
    Subfolders(bool),
    Choose,
    Chosen(Option<PathBuf>),
    Search,
    Done(usize, Result<Vec<Found>, String>),
    Stop,
    /// Opens file `file` of the results at `page`.
    Show(PathBuf, usize),
    Close,
}

impl FolderSearch {
    /// Stops searching, keeping what was found.
    pub fn stop(&mut self) {
        if let Some(run) = &mut self.run {
            run.stopped = true;
        }
    }

    pub fn running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| !r.finished())
    }

    fn can_search(&self) -> bool {
        self.folder.is_some() && !self.query.trim().is_empty() && !self.running()
    }

    /// Handles `event`; the card closes when this returns `None`.
    pub fn update(mut self, event: Event) -> (Option<Self>, Task<Message>) {
        match event {
            Event::Open => {}
            Event::Query(q) => self.query = q.chars().take(500).collect(),
            Event::MatchCase(on) => self.options.match_case = on,
            Event::WholeWord(on) => self.options.whole_word = on,
            Event::Subfolders(on) => self.subfolders = on,
            Event::Choose => {
                let task = Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Choose the folder of PDFs to search")
                            .pick_folder()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    |dir| Message::FolderSearch(Event::Chosen(dir)),
                );
                return (Some(self), task);
            }
            Event::Chosen(None) => {}
            Event::Chosen(Some(dir)) => {
                self.folder = Some(dir);
                if self.can_search() {
                    return self.update(Event::Search);
                }
            }
            Event::Search => {
                if !self.can_search() {
                    return (Some(self), Task::none());
                }
                let Some(folder) = self.folder.clone() else {
                    return (Some(self), Task::none());
                };
                let (files, failed) = match panna_core::folder::pdfs_under(&folder, self.subfolders) {
                    Ok(files) => (files, Vec::new()),
                    Err(e) => (
                        Vec::new(),
                        vec![(
                            folder.display().to_string(),
                            format!("The folder couldn't be read: {e}"),
                        )],
                    ),
                };
                self.run = Some(Run {
                    query: self.query.trim().to_owned(),
                    files,
                    done: 0,
                    found: Vec::new(),
                    failed,
                    stopped: false,
                });
                let task = self.next();
                return (Some(self), task);
            }
            Event::Done(index, result) => {
                if let Some(run) = &mut self.run
                    && index == run.done
                {
                    let file = run.files.get(index).cloned().unwrap_or_default();
                    match result {
                        Ok(found) if !found.is_empty() => run.found.push((file, found)),
                        Ok(_) => {}
                        Err(e) => run.failed.push((
                            file.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            e,
                        )),
                    }
                    run.done += 1;
                }
                let task = self.next();
                return (Some(self), task);
            }
            Event::Stop => {
                if let Some(run) = &mut self.run {
                    run.stopped = true;
                }
            }
            Event::Show(file, page) => {
                let query = self.run.as_ref().map(|r| r.query.clone()).unwrap_or_default();
                let options = self.options;
                if let Some(run) = &mut self.run {
                    run.stopped = true;
                }
                return (
                    None,
                    Task::done(Message::OpenFoundAt {
                        path: file,
                        page,
                        query,
                        options,
                    }),
                );
            }
            Event::Close => {
                if let Some(run) = &mut self.run {
                    run.stopped = true;
                }
                return (None, Task::none());
            }
        }
        (Some(self), Task::none())
    }

    /// Starts on the next file, if there is one and it hasn't been stopped.
    fn next(&self) -> Task<Message> {
        let Some(run) = self.run.as_ref().filter(|r| !r.finished()) else {
            return Task::none();
        };
        let Some(file) = run.files.get(run.done).cloned() else {
            return Task::none();
        };
        let (index, query, options) = (run.done, run.query.clone(), self.options);
        let (tx, rx) = futures_channel::oneshot::channel();
        std::thread::spawn(move || {
            let result = panna_core::folder::search(&file, &query, options, panna_core::Limits::default());
            let _ = tx.send(result);
        });
        Task::perform(rx, move |result| {
            Message::FolderSearch(Event::Done(
                index,
                result.unwrap_or_else(|_| Err("It stopped unexpectedly.".into())),
            ))
        })
    }
}

pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("folder-search")
}

/// The Search a folder card.
pub fn card(state: &FolderSearch) -> Element<'_, Message> {
    let ev = Message::FolderSearch;
    let running = state.running();
    let folder = state
        .folder
        .as_ref()
        .map_or_else(|| "No folder chosen".to_owned(), |f| f.display().to_string());
    let mut body = column![
        text("Search a folder of PDFs").size(16),
        row![
            icons::glyph::<Message>(Icon::Search, true),
            text_input("Words to find", &state.query)
                .id(input_id())
                .on_input(move |q| ev(Event::Query(q)))
                .on_submit(ev(Event::Search))
                .padding([5, 8]),
        ]
        .spacing(6)
        .align_y(Center),
        row![
            text(folder).size(13).width(Fill),
            button("Choose folder…")
                .style(button::secondary)
                .on_press_maybe((!running).then_some(ev(Event::Choose))),
        ]
        .spacing(8)
        .align_y(Center),
        row![
            checkbox(state.subfolders)
                .label("Include folders inside it")
                .on_toggle(move |v| ev(Event::Subfolders(v))),
            checkbox(state.options.match_case)
                .label("Match case")
                .on_toggle(move |v| ev(Event::MatchCase(v))),
            checkbox(state.options.whole_word)
                .label("Whole words")
                .on_toggle(move |v| ev(Event::WholeWord(v))),
        ]
        .spacing(14),
    ]
    .spacing(10)
    .width(560);

    if let Some(run) = &state.run {
        let total = run.files.len();
        let matches: usize = run.found.iter().map(|(_, f)| f.len()).sum();
        let status = if total == 0 && run.failed.is_empty() {
            "There are no PDFs in that folder.".to_owned()
        } else if !run.finished() {
            format!(
                "Searching {} of {total} files… {matches} {} so far",
                run.done + 1,
                if matches == 1 { "match" } else { "matches" }
            )
        } else {
            let stopped = if run.stopped && run.done < total {
                " (stopped)"
            } else {
                ""
            };
            format!(
                "{matches} {} in {} of {} files{stopped}",
                if matches == 1 { "match" } else { "matches" },
                run.found.len(),
                run.done
            )
        };
        body = body.push(text(status).size(13));
        let mut list = Column::new().spacing(2).width(Fill);
        let mut listed = 0;
        for (file, found) in &run.found {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            list = list.push(
                row![
                    icons::glyph::<Message>(Icon::SinglePage, true),
                    text(name).size(13),
                    text(format!("({})", found.len())).size(12).style(text::secondary),
                ]
                .spacing(6)
                .align_y(Center),
            );
            for hit in found {
                if listed >= MAX_LISTED {
                    break;
                }
                listed += 1;
                let snippet: text::Rich<'_, (), Message> = rich_text![
                    span(hit.before.as_str()),
                    span(hit.matched.as_str()).background(MATCH).color(Color::BLACK),
                    span(hit.after.as_str()),
                ]
                .size(13)
                .wrapping(text::Wrapping::WordOrGlyph);
                list = list.push(
                    button(
                        column![
                            text(format!("Page {}", hit.page + 1))
                                .size(11)
                                .style(text::secondary),
                            snippet
                        ]
                        .spacing(2),
                    )
                    .width(Fill)
                    .padding([4, 24])
                    .style(button::text)
                    .on_press(ev(Event::Show(file.clone(), hit.page))),
                );
            }
        }
        if listed >= MAX_LISTED {
            list = list.push(
                text(format!("Showing the first {MAX_LISTED} matches."))
                    .size(12)
                    .style(text::secondary),
            );
        }
        if !run.failed.is_empty() {
            list = list.push(text("These weren't searched:").size(13));
            for (name, why) in &run.failed {
                list = list.push(text(format!("{name}: {why}")).size(12).style(text::secondary));
            }
        }
        body = body.push(container(scrollable(list)).height(320));
    }
    body = body.push(
        row![
            space::horizontal(),
            if running {
                button("Stop").style(button::secondary).on_press(ev(Event::Stop))
            } else {
                button("Close")
                    .style(button::secondary)
                    .on_press(ev(Event::Close))
            },
            button("Search").on_press_maybe(state.can_search().then_some(ev(Event::Search))),
        ]
        .spacing(8),
    );
    container(body).padding(16).style(container::rounded_box).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_are_kept_for_files_with_matches_and_failures_named() {
        let mut state = FolderSearch {
            query: "x".into(),
            folder: Some(PathBuf::from("/nowhere")),
            ..FolderSearch::default()
        };
        state.run = Some(Run {
            query: "x".into(),
            files: vec!["a.pdf".into(), "b.pdf".into(), "c.pdf".into()],
            done: 0,
            found: Vec::new(),
            failed: Vec::new(),
            stopped: false,
        });
        let hit = Found {
            page: 2,
            before: String::new(),
            matched: "x".into(),
            after: String::new(),
        };
        // Finished files are taken in order; a late answer for another file is ignored.
        let (state, _) = state.update(Event::Done(0, Ok(vec![hit.clone()])));
        let (state, _) = state.unwrap().update(Event::Done(5, Ok(vec![hit.clone()])));
        let (state, _) = state.unwrap().update(Event::Done(1, Ok(Vec::new())));
        let (state, _) = state.unwrap().update(Event::Done(2, Err("locked".into())));
        let state = state.unwrap();
        let run = state.run.as_ref().unwrap();
        assert!(run.finished());
        assert_eq!(run.found.len(), 1);
        assert_eq!(run.found[0].0, PathBuf::from("a.pdf"));
        assert_eq!(run.failed, [("c.pdf".to_owned(), "locked".to_owned())]);
        assert!(!state.running());
    }
}
