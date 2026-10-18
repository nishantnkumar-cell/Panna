//! The system print dialogs on Windows and macOS.
//!
//! Both are reachable only through OS calls that Rust can't check, so this is the one crate in
//! Panna that may use `unsafe`, and only in the module for each system. Everything else here,
//! such as where a page goes on the paper, is ordinary safe code. On Linux, Panna prints through
//! the desktop's print portal instead, which needs none of this.

/// A page's size in points (1/72 inch), as it is shown, its turn included.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Page {
    pub width: f32,
    pub height: f32,
}

/// A rendered page: `width` × `height` pixels of RGBA, row by row from the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Printed,
    Cancelled,
}

/// What to print.
pub struct Job<'a> {
    /// The name the print queue shows.
    pub title: &'a str,
    /// The document as a PDF without a password. macOS prints this through PDFKit.
    pub pdf: &'a [u8],
    /// Every page's size. Windows draws each page itself, using `render`.
    pub pages: &'a [Page],
    /// The page on screen, for "Current page".
    pub current: usize,
    /// The window the dialog belongs to (a Windows HWND).
    pub owner: Option<isize>,
    /// Renders page `index` at `scale` pixels per point, unturned.
    pub render: &'a mut dyn FnMut(usize, f32) -> Option<Bitmap>,
}

/// Shows the system print dialog, and prints if the user goes ahead. Blocks until done.
pub fn print(job: Job<'_>) -> Result<Outcome, String> {
    system::print(job)
}

/// Finer than this adds nothing a printer can show of a rendered page, and costs memory.
#[cfg_attr(not(windows), allow(dead_code))]
const MAX_DPI: f32 = 300.0;

/// Where a page goes in the printable area, in device pixels.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placement {
    /// Turned a quarter clockwise to match the paper's orientation.
    turn: bool,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    /// Pixels per point to render the page at.
    scale: f32,
}

/// Puts `page` on paper `paper` pixels in size with its printable area at `offset` and `area`
/// in size, at `dpi`: turned to match the paper, actual size if it fits, otherwise shrunk to
/// fit, and centred on the paper as far as the printable area allows.
#[cfg_attr(not(windows), allow(dead_code))]
fn place(page: Page, paper: (i32, i32), offset: (i32, i32), area: (i32, i32), dpi: (f32, f32)) -> Placement {
    let landscape = |w: f32, h: f32| w > h;
    let turn = page.width != page.height
        && landscape(page.width, page.height) != landscape(area.0 as f32, area.1 as f32);
    let (pw, ph) = if turn {
        (page.height, page.width)
    } else {
        (page.width, page.height)
    };
    let actual = (pw / 72.0 * dpi.0, ph / 72.0 * dpi.1);
    let fit = (area.0 as f32 / actual.0)
        .min(area.1 as f32 / actual.1)
        .clamp(0.0, 1.0);
    let (width, height) = ((actual.0 * fit).round() as i32, (actual.1 * fit).round() as i32);
    let centred = |paper: i32, offset: i32, area: i32, size: i32| {
        ((paper - size) / 2 - offset).clamp(0, (area - size).max(0))
    };
    Placement {
        turn,
        x: centred(paper.0, offset.0, area.0, width),
        y: centred(paper.1, offset.1, area.1, height),
        width,
        height,
        scale: dpi.0.min(dpi.1).min(MAX_DPI) / 72.0 * fit,
    }
}

/// `bitmap` turned a quarter clockwise.
#[cfg_attr(not(windows), allow(dead_code))]
fn turn(bitmap: &Bitmap) -> Bitmap {
    let (w, h) = (bitmap.width as usize, bitmap.height as usize);
    let mut rgba = vec![0; bitmap.rgba.len()];
    for (i, pixel) in bitmap.rgba.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (i % w, i / w);
        // Row y becomes column h - 1 - y.
        let at = (x * h + (h - 1 - y)) * 4;
        if let Some(slot) = rgba.get_mut(at..at + 4) {
            slot.copy_from_slice(pixel);
        }
    }
    Bitmap {
        width: bitmap.height,
        height: bitmap.width,
        rgba,
    }
}

/// The pages to print from 1-based inclusive `ranges`, in order, ignoring any past `count`.
#[cfg_attr(not(windows), allow(dead_code))]
fn pages_in(ranges: &[(u32, u32)], count: usize) -> Vec<usize> {
    ranges
        .iter()
        .flat_map(|&(from, to)| from.min(to)..=from.max(to))
        .filter_map(|n| usize::try_from(n).ok()?.checked_sub(1))
        .filter(|&i| i < count)
        .collect()
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod system {
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{GlobalFree, HWND, S_OK};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps, HALFTONE, HDC,
        HORZRES, LOGPIXELSX, LOGPIXELSY, PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY, PHYSICALWIDTH,
        SRCCOPY, SetBrushOrgEx, SetStretchBltMode, StretchDIBits, VERTRES,
    };
    use windows_sys::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
    use windows_sys::Win32::UI::Controls::Dialogs::{
        PD_CURRENTPAGE, PD_NOSELECTION, PD_PAGENUMS, PD_RESULT_PRINT, PD_RETURNDC,
        PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGEXW, PRINTPAGERANGE, PrintDlgExW, START_PAGE_GENERAL,
    };

    use super::{Bitmap, Job, Outcome, Placement, pages_in, place, turn};

    const MAX_RANGES: usize = 32;

    /// Frees what the print dialog hands back, whatever happens after it.
    struct Dialog(PRINTDLGEXW);

    impl Drop for Dialog {
        fn drop(&mut self) {
            // SAFETY: the handles are null or were returned by PrintDlgExW for us to free, and
            // are freed once, here.
            unsafe {
                if !self.0.hDevMode.is_null() {
                    GlobalFree(self.0.hDevMode);
                }
                if !self.0.hDevNames.is_null() {
                    GlobalFree(self.0.hDevNames);
                }
                if !self.0.hDC.is_null() {
                    DeleteDC(self.0.hDC);
                }
            }
        }
    }

    pub fn print(job: Job<'_>) -> Result<Outcome, String> {
        let count = u32::try_from(job.pages.len()).map_err(|_| "Too many pages to print.".to_owned())?;
        if count == 0 {
            return Ok(Outcome::Cancelled);
        }
        let mut ranges = [PRINTPAGERANGE {
            nFromPage: 1,
            nToPage: count,
        }; MAX_RANGES];
        // SAFETY: PRINTDLGEXW is plain data, for which all zeroes is a valid empty value.
        let mut dialog = Dialog(unsafe { std::mem::zeroed() });
        let d = &mut dialog.0;
        d.lStructSize = std::mem::size_of::<PRINTDLGEXW>() as u32;
        d.hwndOwner = job.owner.unwrap_or(0) as HWND;
        d.Flags = PD_RETURNDC | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE;
        d.nMaxPageRanges = MAX_RANGES as u32;
        d.lpPageRanges = ranges.as_mut_ptr();
        d.nMinPage = 1;
        d.nMaxPage = count;
        d.nCopies = 1;
        d.nStartPage = START_PAGE_GENERAL;
        // SAFETY: `d` is a correctly sized PRINTDLGEXW, and `ranges` has room for the
        // nMaxPageRanges entries the dialog may write; both outlive the call.
        let result = unsafe { PrintDlgExW(d) };
        if result != S_OK {
            return Err(format!("The print dialog couldn't open (error {result:#x})."));
        }
        if d.dwResultAction != PD_RESULT_PRINT {
            return Ok(Outcome::Cancelled);
        }
        let pages = if d.Flags & PD_PAGENUMS != 0 {
            let chosen = ranges.get(..d.nPageRanges as usize).unwrap_or(&[]);
            let pairs: Vec<(u32, u32)> = chosen.iter().map(|r| (r.nFromPage, r.nToPage)).collect();
            pages_in(&pairs, job.pages.len())
        } else if d.Flags & PD_CURRENTPAGE != 0 {
            vec![job.current.min(job.pages.len() - 1)]
        } else {
            (0..job.pages.len()).collect()
        };
        if d.hDC.is_null() {
            return Err("The printer couldn't be prepared.".into());
        }
        print_pages(d.hDC, job.title, &pages, job.pages, job.render)
    }

    fn print_pages(
        hdc: HDC,
        title: &str,
        pages: &[usize],
        sizes: &[super::Page],
        render: &mut dyn FnMut(usize, f32) -> Option<Bitmap>,
    ) -> Result<Outcome, String> {
        // SAFETY: `hdc` is the printer device context the dialog returned, valid until the
        // dialog is dropped after this returns.
        let caps = |index: u32| unsafe { GetDeviceCaps(hdc, index as i32) };
        let dpi = (caps(LOGPIXELSX) as f32, caps(LOGPIXELSY) as f32);
        let paper = (caps(PHYSICALWIDTH), caps(PHYSICALHEIGHT));
        let offset = (caps(PHYSICALOFFSETX), caps(PHYSICALOFFSETY));
        let area = (caps(HORZRES), caps(VERTRES));
        if dpi.0 <= 0.0 || dpi.1 <= 0.0 || area.0 <= 0 || area.1 <= 0 {
            return Err("The printer didn't say its page size.".into());
        }
        let name: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let info = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: name.as_ptr(),
            lpszOutput: null(),
            lpszDatatype: null(),
            fwType: 0,
        };
        // SAFETY: `info` and the name it points to outlive the call.
        if unsafe { StartDocW(hdc, &info) } <= 0 {
            return Err("The printer couldn't start the job.".into());
        }
        let failed = |message: &str| {
            // SAFETY: a document was started on `hdc` and is abandoned once.
            unsafe { AbortDoc(hdc) };
            Err(message.to_owned())
        };
        for &index in pages {
            let Some(&size) = sizes.get(index) else { continue };
            // SAFETY: a document is started on `hdc`.
            if unsafe { StartPage(hdc) } <= 0 {
                return failed("The printer stopped taking pages.");
            }
            let at = place(size, paper, offset, area, dpi);
            match render(index, at.scale) {
                Some(bitmap) if at.turn => draw(hdc, &turn(&bitmap), at),
                Some(bitmap) => draw(hdc, &bitmap, at),
                None => return failed(&format!("Page {} couldn't be prepared for printing.", index + 1)),
            }
            // SAFETY: a page is started on `hdc`.
            if unsafe { EndPage(hdc) } <= 0 {
                return failed("The printer stopped taking pages.");
            }
        }
        // SAFETY: a document is started on `hdc` and ends once.
        unsafe { EndDoc(hdc) };
        Ok(Outcome::Printed)
    }

    /// Draws `bitmap` stretched to `at`, as a 24-bit bottom-to-top-free DIB, which every
    /// printer driver takes.
    fn draw(hdc: HDC, bitmap: &Bitmap, at: Placement) {
        let (w, h) = (bitmap.width as usize, bitmap.height as usize);
        // Rows of BGR, each padded to a multiple of four bytes.
        let stride = (w * 3).div_ceil(4) * 4;
        let mut bgr = vec![255u8; stride * h];
        for (y, row) in bitmap.rgba.chunks(w * 4).take(h).enumerate() {
            let Some(out) = bgr.get_mut(y * stride..y * stride + w * 3) else {
                break;
            };
            for (dst, src) in out.as_chunks_mut::<3>().0.iter_mut().zip(row.as_chunks::<4>().0) {
                // Blend onto white paper.
                let [r, g, b, a] = src.map(u32::from);
                let over = |c: u32| ((c * a + 255 * (255 - a)) / 255) as u8;
                *dst = [over(b), over(g), over(r)];
            }
        }
        // SAFETY: BITMAPINFO is plain data, for which all zeroes is a valid empty value.
        let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            // Negative: rows run from the top.
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 24,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        // SAFETY: `bgr` holds `h` rows of `stride` bytes as `info` describes, and both outlive
        // the calls; `hdc` is valid with a page started.
        unsafe {
            SetStretchBltMode(hdc, HALFTONE);
            SetBrushOrgEx(hdc, 0, 0, null_mut());
            StretchDIBits(
                hdc,
                at.x,
                at.y,
                at.width,
                at.height,
                0,
                0,
                w as i32,
                h as i32,
                bgr.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod system {
    use objc2::AllocAnyThread;
    use objc2::MainThreadMarker;
    use objc2_foundation::{NSData, NSString};
    use objc2_pdf_kit::{PDFDocument, PDFPrintScalingMode};

    use super::{Job, Outcome};

    /// PDFKit's own print panel, with its preview, scaling the pages down to fit and turning
    /// them to match the paper.
    pub fn print(job: Job<'_>) -> Result<Outcome, String> {
        // The print panel is AppKit's, which runs only on the main thread.
        let Some(main) = MainThreadMarker::new() else {
            return Err("Printing has to start from the main window.".into());
        };
        let data = NSData::with_bytes(job.pdf);
        // SAFETY: `data` is a valid NSData holding the PDF; PDFKit copies what it needs.
        let document = unsafe { PDFDocument::initWithData(PDFDocument::alloc(), &data) }
            .ok_or_else(|| "The pages couldn't be prepared for printing.".to_owned())?;
        // SAFETY: on the main thread (`main`), with a valid document; no print info means the
        // shared one.
        let operation = unsafe {
            document.printOperationForPrintInfo_scalingMode_autoRotate(
                None,
                PDFPrintScalingMode::PageScaleDownToFit,
                true,
                main,
            )
        }
        .ok_or_else(|| "The print dialog couldn't open.".to_owned())?;
        operation.setJobTitle(Some(&NSString::from_str(job.title)));
        let printed = operation.runOperation();
        Ok(if printed {
            Outcome::Printed
        } else {
            Outcome::Cancelled
        })
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod system {
    use super::{Job, Outcome};

    pub fn print(_job: Job<'_>) -> Result<Outcome, String> {
        Err("This system prints through the desktop's print portal.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: Page = Page {
        width: 612.0,
        height: 792.0,
    };

    #[test]
    fn a_page_that_fits_prints_at_actual_size_centred() {
        // Letter paper at 600 dpi, printable area inset by 0.25 inch.
        let a5 = Page {
            width: 420.0,
            height: 595.0,
        };
        let at = place(a5, (5100, 6600), (150, 150), (4800, 6300), (600.0, 600.0));
        assert!(!at.turn);
        assert_eq!((at.width, at.height), (3500, 4958));
        assert_eq!((at.x, at.y), ((5100 - 3500) / 2 - 150, (6600 - 4958) / 2 - 150));
        // A full Letter page doesn't fit inside the margins, so it shrinks a little.
        let at = place(LETTER, (5100, 6600), (150, 150), (4800, 6300), (600.0, 600.0));
        assert_eq!(at.width, 4800);
        assert!(at.height <= 6300);
    }

    #[test]
    fn large_pages_shrink_and_wide_pages_turn() {
        let wide = Page {
            width: 1584.0,
            height: 1224.0,
        };
        let at = place(wide, (2550, 3300), (0, 0), (2550, 3300), (300.0, 300.0));
        assert!(at.turn);
        // Turned it is 17 × 22 inches, shrunk to the 8.5-inch width exactly.
        assert_eq!(at.width, 2550);
        assert_eq!(at.height, 3300);
        assert!((at.scale - 300.0 / 72.0 * 0.5).abs() < 1e-4);
    }

    #[test]
    fn small_pages_are_centred_on_the_paper() {
        let card = Page {
            width: 288.0,
            height: 432.0,
        };
        let at = place(card, (2550, 3300), (75, 75), (2400, 3150), (300.0, 300.0));
        assert_eq!((at.width, at.height), (1200, 1800));
        assert_eq!((at.x, at.y), ((2550 - 1200) / 2 - 75, (3300 - 1800) / 2 - 75));
        // Rendering stops at 300 dpi however fine the printer.
        let fine = place(card, (10200, 13200), (0, 0), (10200, 13200), (1200.0, 1200.0));
        assert!((fine.scale - 300.0 / 72.0).abs() < 1e-4);
    }

    #[test]
    fn bitmaps_turn_clockwise() {
        // 2 × 1: red, green.
        let b = Bitmap {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        };
        let t = turn(&b);
        assert_eq!((t.width, t.height), (1, 2));
        assert_eq!(t.rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]);
        // 1 × 2 (top red, bottom green) turned is green on the left, red on the right.
        let t = turn(&t);
        assert_eq!((t.width, t.height), (2, 1));
        assert_eq!(t.rgba, vec![0, 255, 0, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn page_ranges_from_the_dialog() {
        assert_eq!(pages_in(&[(1, 2), (5, 5), (9, 20)], 10), vec![0, 1, 4, 8, 9]);
        assert_eq!(pages_in(&[(0, 1)], 3), vec![0]);
    }
}
