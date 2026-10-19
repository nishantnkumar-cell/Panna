//! What a PDF/A copy adds: metadata saying it is PDF/A-2b, and an sRGB colour profile for
//! its output intent, made here rather than shipped as a file.

/// The XMP metadata of a PDF/A-2b file, with the document's title and author when it has
/// them (the document information dictionary is left out, so the two can't disagree).
pub(crate) fn xmp(title: Option<&str>, author: Option<&str>) -> Vec<u8> {
    let escape = |s: &str| {
        s.chars()
            .filter(|c| !c.is_control())
            .fold(String::new(), |mut out, c| {
                match c {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    '"' => out.push_str("&quot;"),
                    c => out.push(c),
                }
                out
            })
    };
    let mut extra = String::new();
    if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
        extra.push_str(&format!(
            "<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n",
            escape(t)
        ));
    }
    if let Some(a) = author.filter(|a| !a.trim().is_empty()) {
        extra.push_str(&format!(
            "<dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>\n",
            escape(a)
        ));
    }
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         <rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\">\n\
         <pdfaid:part>2</pdfaid:part>\n<pdfaid:conformance>B</pdfaid:conformance>\n\
         {extra}<pdf:Producer>Panna</pdf:Producer>\n\
         </rdf:Description>\n</rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>"
    )
    .into_bytes()
}

fn s15(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

fn xyz(values: [f64; 3]) -> Vec<u8> {
    let mut out = b"XYZ \0\0\0\0".to_vec();
    for v in values {
        out.extend(s15(v));
    }
    out
}

/// An ICC version 2 display profile for sRGB: its primaries (adapted to D50) and its tone
/// curve as a table.
pub(crate) fn srgb_profile() -> Vec<u8> {
    let mut desc = b"desc\0\0\0\0".to_vec();
    let name = b"sRGB (Panna)\0";
    desc.extend((name.len() as u32).to_be_bytes());
    desc.extend(name);
    // No Unicode or ScriptCode description.
    desc.extend([0u8; 4 + 4 + 2 + 1 + 67]);
    let mut cprt = b"text\0\0\0\0".to_vec();
    cprt.extend(b"No copyright, use freely\0");
    let mut curve = b"curv\0\0\0\0".to_vec();
    const POINTS: u32 = 1024;
    curve.extend(POINTS.to_be_bytes());
    for i in 0..POINTS {
        let c = f64::from(i) / f64::from(POINTS - 1);
        let linear = if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        };
        curve.extend(((linear * 65535.0).round() as u16).to_be_bytes());
    }
    let tags: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"desc", desc),
        (b"cprt", cprt),
        (b"wtpt", xyz([0.9642, 1.0, 0.8249])),
        (b"rXYZ", xyz([0.436_074_7, 0.222_504_5, 0.013_932_2])),
        (b"gXYZ", xyz([0.385_064_9, 0.716_878_6, 0.097_104_5])),
        (b"bXYZ", xyz([0.143_080_4, 0.060_616_9, 0.714_173_3])),
        (b"rTRC", curve.clone()),
        (b"gTRC", curve.clone()),
        (b"bTRC", curve),
    ];
    let table_len = 4 + 12 * tags.len();
    let mut data = Vec::new();
    let mut table = (tags.len() as u32).to_be_bytes().to_vec();
    for (sig, body) in &tags {
        let offset = 128 + table_len + data.len();
        table.extend(*sig);
        table.extend((offset as u32).to_be_bytes());
        table.extend((body.len() as u32).to_be_bytes());
        data.extend(body);
        while data.len() % 4 != 0 {
            data.push(0);
        }
    }
    let size = 128 + table.len() + data.len();
    let mut out = Vec::with_capacity(size);
    out.extend((size as u32).to_be_bytes());
    out.extend([0u8; 4]); // preferred CMM
    out.extend([2, 0x10, 0, 0]); // version 2.1
    out.extend(b"mntrRGB XYZ ");
    out.extend([0u8; 12]); // date
    out.extend(b"acsp");
    out.extend([0u8; 4 + 4 + 4 + 4 + 8]); // platform, flags, maker, model, attributes
    out.extend([0u8; 4]); // perceptual intent
    out.extend(s15(0.9642));
    out.extend(s15(1.0));
    out.extend(s15(0.8249));
    out.extend([0u8; 4 + 16 + 28]); // creator, ID, reserved
    out.extend(table);
    out.extend(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_is_well_formed() {
        let p = srgb_profile();
        assert_eq!(u32::from_be_bytes([p[0], p[1], p[2], p[3]]) as usize, p.len());
        assert_eq!(&p[12..24], b"mntrRGB XYZ ");
        assert_eq!(&p[36..40], b"acsp");
        let count = u32::from_be_bytes([p[128], p[129], p[130], p[131]]);
        assert_eq!(count, 9);
        // Every tag lies inside the profile, starting on a 4-byte boundary.
        for t in 0..count as usize {
            let at = 132 + t * 12;
            let offset = u32::from_be_bytes([p[at + 4], p[at + 5], p[at + 6], p[at + 7]]) as usize;
            let len = u32::from_be_bytes([p[at + 8], p[at + 9], p[at + 10], p[at + 11]]) as usize;
            assert_eq!(offset % 4, 0);
            assert!(offset + len <= p.len());
        }
    }

    #[test]
    fn metadata_names_pdfa_and_escapes_text() {
        let x = String::from_utf8(xmp(Some("A & <B>"), None)).unwrap();
        assert!(x.contains("<pdfaid:part>2</pdfaid:part>"));
        assert!(x.contains("A &amp; &lt;B&gt;"));
        assert!(!x.contains("dc:creator"));
    }
}
