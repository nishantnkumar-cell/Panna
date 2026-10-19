//! Drawing pages in a separate process. The app starts a copy of itself with [`ARG`], which
//! locks itself down (no files, no network, limited memory; see the panna-sandbox crate)
//! and then [`serve`]s: it's sent the document's bytes and asked for pages, and sends back
//! bitmaps. A bug in a decoder's `unsafe` code, a page that never finishes, or a page that
//! eats memory then takes down only that process: the page shows as not drawn, and the next
//! page starts a fresh one.
//!
//! Messages both ways are a tag byte, a little-endian `u64` length and that many bytes.

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::time::Duration;

use crate::{Document, Limits, RenderError, RenderedPage};

/// The argument that starts a renderer.
pub const ARG: &str = "--panna-render";

/// How long one page may take to draw before its renderer is stopped.
const PAGE_TIME: Duration = Duration::from_secs(30);
/// How long a document may take to load in a renderer.
const LOAD_TIME: Duration = Duration::from_secs(60);

const LOAD: u8 = 1;
const RENDER: u8 = 2;
const PAGE: u8 = 3;
const FAILED: u8 = 4;
const LOADED: u8 = 5;
const READY: u8 = 6;

/// How long a renderer may take to start and check itself.
const START_TIME: Duration = Duration::from_secs(15);

/// A one-page document with a line of text, which a renderer draws before saying it's ready,
/// to show that it still can once locked down.
const CHECK: &[u8] = b"%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 4 0 R
/Resources << /Font << /F1 5 0 R >> >> >> endobj
4 0 obj << /Length 37 >> stream
BT /F1 12 Tf 2 6 Td (Panna) Tj ET 0 0 m
endstream endobj
5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
trailer << /Root 1 0 R >>
%%EOF
";

static PROGRAM: OnceLock<PathBuf> = OnceLock::new();

/// Draws pages with `program` (normally the app itself), started with [`ARG`]. Without
/// this, pages are drawn in the calling process, as in tests.
pub fn use_renderer(program: PathBuf) {
    let _ = PROGRAM.set(program);
}

fn write_frame(out: &mut impl Write, tag: u8, parts: &[&[u8]]) -> std::io::Result<()> {
    let len: usize = parts.iter().map(|p| p.len()).sum();
    out.write_all(&[tag])?;
    out.write_all(&(len as u64).to_le_bytes())?;
    for part in parts {
        out.write_all(part)?;
    }
    out.flush()
}

fn read_frame(input: &mut impl Read, max: u64) -> std::io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 9];
    input.read_exact(&mut head)?;
    let [tag, len @ ..] = head;
    let len = u64::from_le_bytes(len);
    if len > max {
        return Err(std::io::Error::other("message too large"));
    }
    let mut body = Vec::new();
    input.take(len).read_to_end(&mut body)?;
    if body.len() as u64 != len {
        return Err(std::io::ErrorKind::UnexpectedEof.into());
    }
    Ok((tag, body))
}

fn limits_bytes(l: &Limits) -> Vec<u8> {
    [
        l.max_file_bytes,
        l.max_pages as u64,
        u64::from(l.max_bitmap_side),
        l.max_bitmap_pixels,
        l.max_decoded_bytes,
        l.max_image_pixels,
    ]
    .iter()
    .flat_map(|v| v.to_le_bytes())
    .collect()
}

/// Reads `n` little-endian `u64`s from the front of `bytes`.
fn numbers<const N: usize>(bytes: &[u8]) -> Option<([u64; N], &[u8])> {
    let (head, rest) = bytes.split_at_checked(N * 8)?;
    let mut out = [0u64; N];
    for (slot, chunk) in out.iter_mut().zip(head.as_chunks::<8>().0) {
        *slot = u64::from_le_bytes(*chunk);
    }
    Some((out, rest))
}

fn limits_from(bytes: &[u8]) -> Option<(Limits, &[u8])> {
    let ([file, pages, side, pixels, decoded, image], rest) = numbers::<6>(bytes)?;
    let limits = Limits {
        max_file_bytes: file,
        max_pages: usize::try_from(pages).ok()?,
        max_bitmap_side: u32::try_from(side).ok()?,
        max_bitmap_pixels: pixels,
        max_decoded_bytes: decoded,
        max_image_pixels: image,
    };
    Some((limits, rest))
}

/// A renderer process, and which document it has.
pub(crate) struct Renderer {
    child: Child,
    to: BufWriter<ChildStdin>,
    from: Receiver<std::io::Result<(u8, Vec<u8>)>>,
    loaded: Option<u64>,
}

impl Renderer {
    /// Starts one, if the app said how.
    pub(crate) fn start() -> Option<Self> {
        let program = PROGRAM.get()?;
        let mut child = Command::new(program)
            .arg(ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let to = BufWriter::new(child.stdin.take()?);
        let mut stdout = BufReader::new(child.stdout.take()?);
        // Replies are read on their own thread, so a renderer that stops answering can be
        // given up on.
        let (tx, from) = sync_channel(1);
        std::thread::Builder::new()
            .name("panna-renderer-reader".into())
            .spawn(move || {
                // A bitmap no larger than the limits allow, and a message.
                let max = Limits::default().max_bitmap_pixels.saturating_mul(4) + 64;
                loop {
                    let frame = read_frame(&mut stdout, max);
                    let failed = frame.is_err();
                    if tx.send(frame).is_err() || failed {
                        break;
                    }
                }
            })
            .ok()?;
        let mut renderer = Self {
            child,
            to,
            from,
            loaded: None,
        };
        match renderer.wait(START_TIME) {
            Ok((READY, _)) => Some(renderer),
            // It couldn't lock itself down, or can't draw once it has.
            _ => None,
        }
    }

    fn wait(&mut self, time: Duration) -> Result<(u8, Vec<u8>), ()> {
        match self.from.recv_timeout(time) {
            Ok(Ok(frame)) => Ok(frame),
            Ok(Err(_)) | Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => Err(()),
        }
    }

    /// Draws a page of `doc` (version `version`); `Err(())` when the renderer has to go.
    pub(crate) fn render(
        &mut self,
        doc: &Document,
        version: u64,
        index: usize,
        scale: f32,
    ) -> Result<Result<RenderedPage, RenderError>, ()> {
        if self.loaded != Some(version) {
            self.loaded = None;
            let password = doc.password().as_bytes();
            let limits = limits_bytes(&doc.limits());
            let len = (password.len() as u64).to_le_bytes();
            write_frame(&mut self.to, LOAD, &[&len, password, &limits, doc.bytes()]).map_err(|_| ())?;
            match self.wait(LOAD_TIME)? {
                (LOADED, _) => self.loaded = Some(version),
                _ => return Err(()),
            }
        }
        let request = [
            (index as u64).to_le_bytes(),
            u64::from(scale.to_bits()).to_le_bytes(),
        ]
        .concat();
        write_frame(&mut self.to, RENDER, &[&request]).map_err(|_| ())?;
        match self.wait(PAGE_TIME)? {
            (PAGE, body) => {
                let ([width, height], rgba) = numbers::<2>(&body).ok_or(())?;
                let (width, height) = (
                    u32::try_from(width).map_err(|_| ())?,
                    u32::try_from(height).map_err(|_| ())?,
                );
                if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
                    return Err(());
                }
                Ok(Ok(RenderedPage {
                    width,
                    height,
                    rgba: rgba.to_vec(),
                }))
            }
            (FAILED, body) => Ok(Err(match body.as_slice() {
                [1, rest @ ..] => {
                    let index = numbers::<1>(rest).map_or(index as u64, |([i], _)| i);
                    RenderError::NoSuchPage(usize::try_from(index).unwrap_or(usize::MAX))
                }
                [2] => RenderError::BadPageSize,
                _ => RenderError::EngineCrashed,
            })),
            _ => Err(()),
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What a renderer does, once locked down: loads the documents it's sent and draws the pages
/// asked for, until its input closes. Returns when it should exit.
pub fn serve() {
    let checked = Document::from_bytes(CHECK.to_vec(), None, Limits::default())
        .ok()
        .and_then(|doc| doc.render_page(&doc.new_cache(), 0, 1.0).ok())
        .is_some_and(|page| page.rgba.as_chunks::<4>().0.iter().any(|[r, ..]| *r < 128));
    if !checked || write_frame(&mut std::io::stdout().lock(), READY, &[]).is_err() {
        return;
    }
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut output = BufWriter::new(std::io::stdout().lock());
    let max_load = Limits::default().max_file_bytes + 4096;
    let mut next = read_frame(&mut input, max_load);
    loop {
        let Ok((LOAD, body)) = next else { return };
        let doc = numbers::<1>(&body).and_then(|([pw_len], rest)| {
            let (password, rest) = rest.split_at_checked(usize::try_from(pw_len).ok()?)?;
            let password = std::str::from_utf8(password).ok()?;
            let (limits, bytes) = limits_from(rest)?;
            std::panic::catch_unwind(|| Document::from_bytes(bytes.to_vec(), Some(password), limits).ok())
                .ok()
                .flatten()
        });
        let Some(doc) = doc else {
            let _ = write_frame(&mut output, FAILED, &[&[3]]);
            next = read_frame(&mut input, max_load);
            continue;
        };
        if write_frame(&mut output, LOADED, &[]).is_err() {
            return;
        }
        drop(body);
        let cache = doc.new_cache();
        next = loop {
            let frame = read_frame(&mut input, max_load);
            let Ok((RENDER, request)) = frame else { break frame };
            let Some(([index, scale], _)) = numbers::<2>(&request) else {
                return;
            };
            let scale = f32::from_bits(u32::try_from(scale).unwrap_or(0));
            let page = usize::try_from(index)
                .map_err(|_| RenderError::NoSuchPage(usize::MAX))
                .and_then(|index| doc.render_page(&cache, index, scale));
            let sent = match page {
                Ok(page) => {
                    let head = [
                        u64::from(page.width).to_le_bytes(),
                        u64::from(page.height).to_le_bytes(),
                    ]
                    .concat();
                    write_frame(&mut output, PAGE, &[&head, &page.rgba])
                }
                Err(RenderError::NoSuchPage(i)) => {
                    write_frame(&mut output, FAILED, &[&[1], &(i as u64).to_le_bytes()])
                }
                Err(RenderError::BadPageSize) => write_frame(&mut output, FAILED, &[&[2]]),
                Err(_) => write_frame(&mut output, FAILED, &[&[0]]),
            };
            if sent.is_err() {
                return;
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_check_page_draws_its_text() {
        let doc =
            Document::from_bytes(CHECK.to_vec(), None, Limits::default()).unwrap_or_else(|e| panic!("{e}"));
        let page = doc
            .render_page(&doc.new_cache(), 0, 1.0)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(page.rgba.as_chunks::<4>().0.iter().any(|[r, ..]| *r < 128));
    }

    #[test]
    fn frames_and_limits_go_through_unchanged() {
        let mut buf = Vec::new();
        write_frame(&mut buf, RENDER, &[b"ab", b"cd"]).unwrap_or_default();
        let (tag, body) = read_frame(&mut buf.as_slice(), 10).unwrap_or_default();
        assert_eq!((tag, body.as_slice()), (RENDER, &b"abcd"[..]));
        assert!(read_frame(&mut buf.as_slice(), 3).is_err());
        assert!(read_frame(&mut &buf[..8], 10).is_err());
        let limits = Limits::default();
        let bytes = [limits_bytes(&limits), b"rest".to_vec()].concat();
        let (back, rest) = limits_from(&bytes).unwrap_or_else(|| panic!("not read"));
        assert_eq!((back, rest), (limits, &b"rest"[..]));
    }
}
