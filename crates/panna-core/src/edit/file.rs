//! Files attached to a page as comments (Acrobat's Attach File tool).

use std::io::Write as _;
use std::sync::Arc;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::comments::ObjRef;

/// The largest file accepted.
pub const MAX_ATTACHMENT_BYTES: usize = 100 * 1024 * 1024;

/// A file attached to a page: a new one, carried compressed, or one already in the file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AttachedFile {
    /// Its file name.
    pub name: String,
    /// Its size in bytes, when known.
    pub size: Option<u64>,
    source: Source,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum Source {
    /// Its contents, zlib-compressed.
    New(Arc<Vec<u8>>),
    /// The file specification already in the document.
    Saved(ObjRef),
}

impl PartialEq for AttachedFile {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.size == other.size
            && match (&self.source, &other.source) {
                (Source::New(a), Source::New(b)) => Arc::ptr_eq(a, b),
                (Source::Saved(a), Source::Saved(b)) => a == b,
                _ => false,
            }
    }
}

impl AttachedFile {
    /// A new attachment of `bytes`, named `name` (without any folder).
    pub fn new(name: &str, bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(format!(
                "That file is too big to attach (the most is {} MB).",
                MAX_ATTACHMENT_BYTES / (1024 * 1024)
            ));
        }
        let name: String = name.chars().filter(|c| !c.is_control()).take(255).collect();
        let name = if name.trim().is_empty() {
            "Attachment".to_owned()
        } else {
            name
        };
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(bytes)
            .map_err(|e| format!("The file couldn't be compressed: {e}"))?;
        let data = encoder
            .finish()
            .map_err(|e| format!("The file couldn't be compressed: {e}"))?;
        Ok(Self {
            name,
            size: Some(bytes.len() as u64),
            source: Source::New(Arc::new(data)),
        })
    }

    /// An attachment already in the document, by its file specification.
    pub(crate) fn saved(name: String, size: Option<u64>, spec: ObjRef) -> Self {
        Self {
            name,
            size,
            source: Source::Saved(spec),
        }
    }

    /// The embedded file stream for a new attachment; `None` for one already saved.
    pub(crate) fn stream(&self, date: &str) -> Option<Vec<u8>> {
        let Source::New(data) = &self.source else {
            return None;
        };
        let mut body = format!(
            "<< /Type /EmbeddedFile /Filter /FlateDecode /Length {} /Params << /Size {} /ModDate ({date}) >> >>\nstream\n",
            data.len(),
            self.size.unwrap_or_default(),
        )
        .into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        Some(body)
    }

    /// The value of the annotation's /FS: a new file specification pointing at `stream_ref`,
    /// or a reference to the one already saved.
    pub(crate) fn spec(&self, stream_ref: Option<&str>) -> String {
        match (&self.source, stream_ref) {
            (Source::Saved(r), _) => format!("{} {} R", r.number, r.generation),
            (Source::New(_), Some(stream)) => {
                let name = super::annot::text_string(&self.name);
                format!("<< /Type /Filespec /F {name} /UF {name} /EF << /F {stream} /UF {stream} >> >>")
            }
            (Source::New(_), None) => "null".to_owned(),
        }
    }
}
