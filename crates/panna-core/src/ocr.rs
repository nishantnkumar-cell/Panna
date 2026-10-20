//! Recognising text in pictures of pages (OCR), offline, with the ocrs engine and the models
//! built into the app.

use std::sync::OnceLock;

use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem as _};
use rten::Model;

static DETECTION: &[u8] = include_bytes!("../models/text-detection.rten");
static RECOGNITION: &[u8] = include_bytes!("../models/text-recognition.rten");

/// A word recognised in a picture, and its box there in pixels: left, top, right, bottom.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Word {
    pub text: String,
    pub rect: [f64; 4],
}

fn engine() -> Result<&'static OcrEngine, String> {
    static ENGINE: OnceLock<Result<OcrEngine, String>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let detection = Model::load_static_slice(DETECTION).map_err(|e| e.to_string())?;
            let recognition = Model::load_static_slice(RECOGNITION).map_err(|e| e.to_string())?;
            OcrEngine::new(OcrEngineParams {
                detection_model: Some(detection),
                recognition_model: Some(recognition),
                ..OcrEngineParams::default()
            })
            .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The words in an RGB picture `width` by `height` pixels, line by line.
pub(crate) fn recognise(rgb: &[u8], width: u32, height: u32) -> Result<Vec<Word>, String> {
    let engine = engine()?;
    let source = ImageSource::from_bytes(rgb, (width, height)).map_err(|e| e.to_string())?;
    let input = engine.prepare_input(source).map_err(|e| e.to_string())?;
    let found = engine.detect_words(&input).map_err(|e| e.to_string())?;
    let lines = engine.find_text_lines(&input, &found);
    let lines = engine.recognize_text(&input, &lines).map_err(|e| e.to_string())?;
    let mut words = Vec::new();
    for line in lines.iter().flatten() {
        for word in line.words() {
            let text = word.to_string();
            let text = text.trim();
            // Single stray marks are usually specks, not text.
            if text.is_empty() || (text.chars().count() == 1 && !text.chars().all(char::is_alphanumeric)) {
                continue;
            }
            let r = word.bounding_rect();
            words.push(Word {
                text: text.to_owned(),
                rect: [
                    f64::from(r.left()),
                    f64::from(r.top()),
                    f64::from(r.right()),
                    f64::from(r.bottom()),
                ],
            });
        }
    }
    Ok(words)
}
