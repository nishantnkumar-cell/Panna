//! Reading PDFs in a separate process. The app starts a copy of itself with [`ARG`], which
//! locks itself down (no files, no network, limited memory; see the panna-sandbox crate)
//! and then [`serve`]s: it's sent the document's bytes and asked for pages, which it sends
//! back as bitmaps, or for anything else the app needs of a document (see `remote`). A bug
//! in a decoder's `unsafe` code, a request that never finishes, or one that eats memory then
//! takes down only that process: the request fails, and the next one starts a fresh one.
//!
//! Messages both ways are a tag byte, a little-endian `u64` length and that many bytes;
//! requests other than pages, and their answers, are postcard-encoded. Answers come from the
//! locked-down process, so the app treats them as untrusted, like the PDF itself.

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::time::Duration;

use crate::remote::{Answer, Doc, Op};
use crate::{Document, Limits, ObjRef, RenderError, RenderedPage};

/// The argument that starts a renderer.
pub const ARG: &str = "--panna-render";

/// How long one page may take to draw before its renderer is stopped.
const PAGE_TIME: Duration = Duration::from_secs(30);
/// How long a document may take to load in a renderer.
const LOAD_TIME: Duration = Duration::from_secs(60);
/// How long anything else asked of a document may take (recognising text in many pages,
/// compacting a large file).
const CALL_TIME: Duration = Duration::from_secs(15 * 60);

const LOAD: u8 = 1;
const RENDER: u8 = 2;
const PAGE: u8 = 3;
const FAILED: u8 = 4;
const LOADED: u8 = 5;
const READY: u8 = 6;
const CALL: u8 = 7;
const ANSWER: u8 = 8;

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
    /// Kept so the process lives as long as this, and stops when this is dropped.
    _child: panna_sandbox::Contained,
    to: BufWriter<Box<dyn std::io::Write + Send>>,
    from: Receiver<std::io::Result<(u8, Vec<u8>)>>,
    /// The version it has, and the layers hidden in it.
    loaded: Option<(u64, Option<Vec<ObjRef>>)>,
}

impl Renderer {
    /// Starts one, if the app said how.
    pub(crate) fn start() -> Option<Self> {
        let program = PROGRAM.get()?;
        let child = panna_sandbox::spawn(program, ARG).ok()?;
        let contained = child.app_container;
        match Self::ready(child) {
            Some(renderer) => Some(renderer),
            // An AppContainer can be refused things a plain process isn't (a policy, an
            // older system); the renderer still locks itself down without one.
            None if contained => Self::ready(panna_sandbox::spawn_plain(program, ARG).ok()?),
            None => None,
        }
    }

    /// The renderer `child` once it says it's ready, or `None` (and it stopped).
    fn ready(mut child: panna_sandbox::Contained) -> Option<Self> {
        let to = BufWriter::new(std::mem::replace(&mut child.stdin, Box::new(std::io::sink())));
        let mut stdout = BufReader::new(std::mem::replace(&mut child.stdout, Box::new(std::io::empty())));
        // Replies are read on their own thread, so a renderer that stops answering can be
        // given up on.
        let (tx, from) = sync_channel(1);
        std::thread::Builder::new()
            .name("panna-renderer-reader".into())
            .spawn(move || {
                // A bitmap no larger than the limits allow, or a new version of a file.
                let limits = Limits::default();
                let max = (limits.max_bitmap_pixels.saturating_mul(4) + 64)
                    .max(limits.max_file_bytes.saturating_mul(2));
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
            _child: child,
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

    /// Sends `doc`, shown with `hidden` layers hidden, unless it has it already. `Err(true)`
    /// when the renderer has to go, `Err(false)` when it couldn't open the document.
    fn load(&mut self, doc: &Doc, hidden: Option<&[ObjRef]>) -> Result<(), bool> {
        let key = (doc.version, hidden.map(<[ObjRef]>::to_vec));
        if self.loaded.as_ref() == Some(&key) {
            return Ok(());
        }
        self.loaded = None;
        let password = doc.password.as_bytes();
        let limits = limits_bytes(&doc.limits);
        let len = (password.len() as u64).to_le_bytes();
        let hidden = postcard::to_stdvec(&key.1).map_err(|_| true)?;
        let hidden_len = (hidden.len() as u64).to_le_bytes();
        write_frame(
            &mut self.to,
            LOAD,
            &[&len, password, &limits, &hidden_len, &hidden, &doc.bytes],
        )
        .map_err(|_| true)?;
        match self.wait(LOAD_TIME).map_err(|()| true)? {
            (LOADED, _) => {
                self.loaded = Some(key);
                Ok(())
            }
            (FAILED, _) => Err(false),
            _ => Err(true),
        }
    }

    /// Does `op` on `doc`; `Err(true)` when the renderer has to go, `Err(false)` when the
    /// document couldn't be read or the request failed.
    pub(crate) fn call(&mut self, doc: &Doc, hidden: Option<&[ObjRef]>, op: &Op) -> Result<Answer, bool> {
        self.load(doc, hidden)?;
        let request = postcard::to_stdvec(op).map_err(|_| false)?;
        write_frame(&mut self.to, CALL, &[&request]).map_err(|_| true)?;
        let time = match op {
            Op::Text(_) => PAGE_TIME,
            Op::Info => LOAD_TIME,
            _ => CALL_TIME,
        };
        match self.wait(time).map_err(|()| true)? {
            (ANSWER, body) => postcard::from_bytes(&body).map_err(|_| true),
            (FAILED, _) => Err(false),
            _ => Err(true),
        }
    }

    /// Draws page `index` of `doc`, shown with `hidden` layers hidden; `Err(())` when the
    /// renderer has to go.
    pub(crate) fn render(
        &mut self,
        doc: &Doc,
        hidden: Option<&[ObjRef]>,
        index: usize,
        scale: f32,
    ) -> Result<Result<RenderedPage, RenderError>, ()> {
        match self.load(doc, hidden) {
            Ok(()) => {}
            Err(false) => return Ok(Err(RenderError::EngineCrashed)),
            Err(true) => return Err(()),
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
        let Ok((LOAD, mut body)) = next else { return };
        let loaded = numbers::<1>(&body).and_then(|([pw_len], rest)| {
            let (password, rest) = rest.split_at_checked(usize::try_from(pw_len).ok()?)?;
            let password = std::str::from_utf8(password).ok()?;
            let (limits, rest) = limits_from(rest)?;
            let ([hidden_len], rest) = numbers::<1>(rest)?;
            let (hidden, bytes) = rest.split_at_checked(usize::try_from(hidden_len).ok()?)?;
            let hidden: Option<Vec<ObjRef>> = postcard::from_bytes(hidden).ok()?;
            std::panic::catch_unwind(|| {
                let doc = Document::from_bytes(bytes.to_vec(), Some(password), limits).ok()?;
                let view = match &hidden {
                    Some(hidden) => Some(doc.with_layers_hidden(hidden).ok()?),
                    None => None,
                };
                Some((doc, view))
            })
            .ok()
            .flatten()
        });
        // The document keeps its own copy of the password, which it wipes when dropped.
        let password_end = numbers::<1>(&body).map_or(0, |([pw_len], _)| 8 + pw_len as usize);
        if let Some(password) = body.get_mut(8..password_end) {
            zeroize::Zeroize::zeroize(password);
        }
        let Some((doc, view)) = loaded else {
            let _ = write_frame(&mut output, FAILED, &[&[3]]);
            next = read_frame(&mut input, max_load);
            continue;
        };
        if write_frame(&mut output, LOADED, &[]).is_err() {
            return;
        }
        drop(body);
        let view = view.as_ref().unwrap_or(&doc);
        let cache = view.new_cache();
        next = loop {
            let frame = read_frame(&mut input, max_load);
            let sent = match frame {
                Ok((RENDER, request)) => {
                    let Some(([index, scale], _)) = numbers::<2>(&request) else {
                        return;
                    };
                    let scale = f32::from_bits(u32::try_from(scale).unwrap_or(0));
                    let page = usize::try_from(index)
                        .map_err(|_| RenderError::NoSuchPage(usize::MAX))
                        .and_then(|index| view.render_page(&cache, index, scale));
                    send_page(&mut output, page)
                }
                Ok((CALL, request)) => {
                    let Ok(op) = postcard::from_bytes::<Op>(&request) else {
                        return;
                    };
                    drop(request);
                    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        postcard::to_stdvec(&crate::remote::perform(&doc, view, &cache, op))
                    }));
                    match answer {
                        Ok(Ok(answer)) => write_frame(&mut output, ANSWER, &[&answer]),
                        _ => write_frame(&mut output, FAILED, &[&[0]]),
                    }
                }
                _ => break frame,
            };
            if sent.is_err() {
                return;
            }
        };
    }
}

fn send_page(output: &mut impl Write, page: Result<RenderedPage, RenderError>) -> std::io::Result<()> {
    match page {
        Ok(page) => {
            let head = [
                u64::from(page.width).to_le_bytes(),
                u64::from(page.height).to_le_bytes(),
            ]
            .concat();
            write_frame(output, PAGE, &[&head, &page.rgba])
        }
        Err(RenderError::NoSuchPage(i)) => write_frame(output, FAILED, &[&[1], &(i as u64).to_le_bytes()]),
        Err(RenderError::BadPageSize) => write_frame(output, FAILED, &[&[2]]),
        Err(_) => write_frame(output, FAILED, &[&[0]]),
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
