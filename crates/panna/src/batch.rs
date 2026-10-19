//! Batch actions (Acrobat's Action Wizard): the same steps over every PDF in a folder, each
//! result saved under its own name in another folder. Files are done one at a time, so the
//! card can show how far it has got and stop between files.

use std::path::PathBuf;

use iced::widget::{button, checkbox, column, container, row, scrollable, space, text, text_input};
use iced::{Center, Element, Task};
use panna_core::batch::Step;

use crate::Message;

#[derive(Debug, Clone)]
pub struct Batch {
    recognise: bool,
    watermark: bool,
    watermark_text: String,
    numbers: bool,
    flatten: bool,
    clean: bool,
    compact: bool,
    /// The files and where they go, once both folders are chosen.
    run: Option<Run>,
}

impl Default for Batch {
    fn default() -> Self {
        Self {
            recognise: false,
            watermark: false,
            watermark_text: "DRAFT".into(),
            numbers: false,
            flatten: false,
            clean: false,
            compact: true,
            run: None,
        }
    }
}

#[derive(Debug, Clone)]
struct Run {
    files: Vec<PathBuf>,
    out: PathBuf,
    steps: Vec<Step>,
    /// Files finished, whether or not they worked.
    done: usize,
    /// Each file that didn't work, with why.
    failed: Vec<(String, String)>,
    stopped: bool,
}

impl Run {
    fn finished(&self) -> bool {
        self.stopped || self.done >= self.files.len()
    }
}

/// The steps that can be ticked, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Recognise,
    Watermark,
    Numbers,
    Flatten,
    Clean,
    Compact,
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Tick(Choice, bool),
    WatermarkText(String),
    /// Asks for the folder of PDFs.
    Start,
    From(Option<PathBuf>),
    To(PathBuf, Option<PathBuf>),
    /// File `index` is finished.
    Done(usize, Result<PathBuf, String>),
    Stop,
    Close,
}

impl Batch {
    fn steps(&self) -> Vec<Step> {
        let mut steps = Vec::new();
        if self.recognise {
            steps.push(Step::Recognise);
        }
        if self.watermark && !self.watermark_text.trim().is_empty() {
            steps.push(Step::Watermark(self.watermark_text.trim().to_owned()));
        }
        if self.numbers {
            steps.push(Step::PageNumbers);
        }
        if self.flatten {
            steps.push(Step::Flatten);
        }
        if self.clean {
            steps.push(Step::Clean);
        }
        if self.compact {
            steps.push(Step::Compact);
        }
        steps
    }

    pub fn running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| !r.finished())
    }

    /// Handles `event`; the card closes when this returns `None`.
    pub fn update(mut self, event: Event) -> (Option<Self>, Task<Message>) {
        match event {
            Event::Open => {}
            Event::Tick(choice, on) => {
                let flag = match choice {
                    Choice::Recognise => &mut self.recognise,
                    Choice::Watermark => &mut self.watermark,
                    Choice::Numbers => &mut self.numbers,
                    Choice::Flatten => &mut self.flatten,
                    Choice::Clean => &mut self.clean,
                    Choice::Compact => &mut self.compact,
                };
                *flag = on;
            }
            Event::WatermarkText(t) => self.watermark_text = t.chars().take(200).collect(),
            Event::Start => {
                self.run = None;
                let task = Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Choose the folder of PDFs")
                            .pick_folder()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    |dir| Message::Batch(Event::From(dir)),
                );
                return (Some(self), task);
            }
            Event::From(None) | Event::To(_, None) => {}
            Event::From(Some(from)) => {
                let task = Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Choose where to save the results")
                            .pick_folder()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    move |to| Message::Batch(Event::To(from.clone(), to)),
                );
                return (Some(self), task);
            }
            Event::To(from, Some(out)) => {
                let (files, failed) = match panna_core::batch::pdfs_in(&from) {
                    Ok(files) => (files, Vec::new()),
                    Err(e) => {
                        let name = from.display().to_string();
                        (
                            Vec::new(),
                            vec![(name, format!("The folder couldn't be read: {e}"))],
                        )
                    }
                };
                self.run = Some(Run {
                    files,
                    out,
                    steps: self.steps(),
                    done: 0,
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
                    if let Err(e) = result {
                        let name = run
                            .files
                            .get(index)
                            .and_then(|f| f.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        run.failed.push((name, e));
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
            Event::Close => {
                if self.running() {
                    return (Some(self), Task::none());
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
        let (index, steps, out) = (run.done, run.steps.clone(), run.out.clone());
        let (tx, rx) = futures_channel::oneshot::channel();
        std::thread::spawn(move || {
            let result = panna_core::batch::run(&file, &steps, &out, panna_core::Limits::default());
            let _ = tx.send(result);
        });
        Task::perform(rx, move |result| {
            Message::Batch(Event::Done(
                index,
                result.unwrap_or_else(|_| Err("It stopped unexpectedly.".into())),
            ))
        })
    }
}

/// The Batch actions card.
pub fn card(state: &Batch) -> Element<'_, Message> {
    let ev = Message::Batch;
    let tick = |label: &'static str, on: bool, choice: Choice| {
        checkbox(on)
            .label(label)
            .on_toggle(move |v| ev(Event::Tick(choice, v)))
    };
    let mut body = column![
        text("Batch actions").size(16),
        text("Runs these steps, in this order, on every PDF in a folder, and saves each result under its own name in another folder.")
            .size(12)
            .style(text::secondary),
        tick("Make scanned pages searchable (OCR)", state.recognise, Choice::Recognise),
        row![
            tick("Add a watermark:", state.watermark, Choice::Watermark),
            text_input("DRAFT", &state.watermark_text)
                .on_input(move |t| ev(Event::WatermarkText(t)))
                .width(150),
        ]
        .spacing(8)
        .align_y(Center),
        tick("Add page numbers (Page 1 of 9)", state.numbers, Choice::Numbers),
        tick("Flatten comments and form fields", state.flatten, Choice::Flatten),
        tick("Remove hidden information and comments", state.clean, Choice::Clean),
        tick("Make the files smaller", state.compact, Choice::Compact),
    ]
    .spacing(10)
    .width(460);
    let has_steps = !state.steps().is_empty();
    match &state.run {
        None => {
            body = body.push(
                row![
                    space::horizontal(),
                    button("Cancel")
                        .style(button::secondary)
                        .on_press(ev(Event::Close)),
                    button("Choose the folder…").on_press_maybe(has_steps.then_some(ev(Event::Start))),
                ]
                .spacing(8),
            );
        }
        Some(run) => {
            let total = run.files.len();
            let status = if total == 0 && run.failed.is_empty() {
                "There are no PDFs in that folder.".to_owned()
            } else if total == 0 {
                String::new()
            } else if !run.finished() {
                let name = run
                    .files
                    .get(run.done)
                    .and_then(|f| f.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                format!("Working on {} of {total}: {name}", run.done + 1)
            } else {
                let worked = run.done.saturating_sub(run.failed.len());
                let stopped = if run.stopped && run.done < total {
                    format!(" Stopped with {} not started.", total - run.done)
                } else {
                    String::new()
                };
                match worked {
                    1 => format!("1 file saved in {}.{stopped}", run.out.display()),
                    n => format!("{n} files saved in {}.{stopped}", run.out.display()),
                }
            };
            body = body.push(text(status));
            if !run.failed.is_empty() {
                let failed = run.failed.iter().fold(column![].spacing(4), |list, (name, why)| {
                    list.push(text(format!("{name}: {why}")).size(12))
                });
                body = body.push(text("These weren't saved:").size(13));
                body = body.push(scrollable(failed).height(120));
            }
            let buttons = if run.finished() {
                row![
                    space::horizontal(),
                    button("Run again…")
                        .style(button::secondary)
                        .on_press_maybe(has_steps.then_some(ev(Event::Start))),
                    button("Done").on_press(ev(Event::Close)),
                ]
            } else {
                row![
                    space::horizontal(),
                    button("Stop").style(button::secondary).on_press(ev(Event::Stop)),
                ]
            };
            body = body.push(buttons.spacing(8));
        }
    }
    container(body).padding(16).style(container::rounded_box).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_the_ticks_in_order_and_skip_an_empty_watermark() {
        let mut b = Batch {
            recognise: true,
            watermark: true,
            watermark_text: "  ".into(),
            numbers: true,
            ..Batch::default()
        };
        assert_eq!(b.steps(), vec![Step::Recognise, Step::PageNumbers, Step::Compact]);
        b.watermark_text = "Copy".into();
        b.compact = false;
        assert_eq!(
            b.steps(),
            vec![Step::Recognise, Step::Watermark("Copy".into()), Step::PageNumbers]
        );
    }

    #[test]
    fn results_count_and_stop_between_files() {
        let mut b = Batch {
            run: Some(Run {
                files: vec!["/a/1.pdf".into(), "/a/2.pdf".into(), "/a/3.pdf".into()],
                out: "/b".into(),
                steps: vec![Step::Compact],
                done: 0,
                failed: Vec::new(),
                stopped: false,
            }),
            ..Batch::default()
        };
        b.run.as_mut().unwrap().stopped = true;
        let (b, _) = b.update(Event::Done(0, Err("broken".into())));
        let b = b.unwrap();
        let run = b.run.as_ref().unwrap();
        assert_eq!(run.done, 1);
        assert_eq!(run.failed, vec![("1.pdf".to_owned(), "broken".to_owned())]);
        assert!(!b.running());
        // A late answer for a file already counted changes nothing.
        let (b, _) = b.update(Event::Done(0, Ok("/b/1.pdf".into())));
        assert_eq!(b.unwrap().run.unwrap().done, 1);
    }
}
