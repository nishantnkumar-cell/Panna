//! Pictures on the system clipboard, for the snapshot tool. iced copies only text.

use std::borrow::Cow;
use std::sync::Mutex;

/// One clipboard for the whole app. On Linux the app itself hands copied pictures to other
/// apps when they paste, so the clipboard has to stay alive after copying.
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

/// Puts an RGBA picture on the clipboard.
pub fn copy_picture(width: u32, height: u32, rgba: Vec<u8>) -> Result<(), String> {
    let mut clipboard = CLIPBOARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if clipboard.is_none() {
        *clipboard = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
    }
    let picture = arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: Cow::Owned(rgba),
    };
    clipboard
        .as_mut()
        .ok_or_else(|| "no clipboard".to_owned())?
        .set_image(picture)
        .map_err(|e| e.to_string())
}
